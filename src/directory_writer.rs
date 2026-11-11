//! **Making and removing directory entries**: files and directories with short or long names, the
//! alias numbered against the names already in the directory, and rename within or across
//! directories.
//!
//! A name that is already a legal upper-case 8.3 name takes one slot. Any other legal name takes a
//! run of long-name slots and then a short entry. Its alias is the plain basis when the name is 8.3
//! but for its case (`readme.txt` is `README.TXT`); no other entry can hold that, since such an
//! entry would already be called the name. Otherwise the alias is numbered `~1`, `~2` and so on
//! until no entry in the directory holds it. The run and its short entry go down in directory
//! order, the short entry last, so a crash leaves orphan pieces that no reader joins and never a
//! short entry with a wrong name. [`RunFinder`] picks the slots; `verification.rs` proves it first-fit.
//!
//! One read of the directory serves a whole create: it refuses a name already there, finds the
//! first run of free slots, and marks in an [`AliasWindow`] every alias number of the new name's
//! basis that an entry holds, so the alias is the window's first free number. Only an alias past
//! the first [`ALIAS_WINDOW`] numbers reads the directory again.

use crate::boot_sector::FileAllocationTableType;
use crate::directory::{SLOT_BYTES, Slot};
use crate::file_writer::{ARCHIVE, FREE, MOST_SLOTS, short_name_bytes};
use crate::long_name::{self, LongName};
use crate::lookup::LookupError;
use crate::named_entry::{EntryPlace, Joiner, NamedEntry};
use crate::reader::{ChainError, DirectoryCluster, Volume};
use crate::sector::{SectorSize, WritableSectorDevice};
use crate::write_cursor::{Medium, WriteError};

/// `DIR_Attr`'s directory bit.
const DIRECTORY: u8 = 0x10;

/// What a directory slot is, for choosing where to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotKind {
    /// In use: a short entry or a long-name piece.
    Used,
    /// Deleted: free, with used slots possibly after it.
    Free,
    /// The end marker: this slot and every slot after it is free.
    End,
}

/// **Finds the first run of `count` free slots** as the slots are fed in directory order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunFinder {
    count: u32,
    at: u32,
    start: u32,
    length_slots: u32,
}

impl RunFinder {
    /// A finder for `count` slots, which is at least 1.
    pub(crate) const fn new(count: u32) -> RunFinder {
        RunFinder {
            count,
            at: 0,
            start: 0,
            length_slots: 0,
        }
    }

    /// **Take the next slot**, and return where the run starts once it is complete. An end marker
    /// completes the run at once, since every slot after it is free too: the caller makes sure the
    /// directory has the slots.
    pub(crate) fn push(&mut self, kind: SlotKind) -> Option<u32> {
        let index = self.at;
        self.at += 1;
        match kind {
            SlotKind::Used => {
                self.length_slots = 0;
                None
            }
            SlotKind::Free | SlotKind::End => {
                if self.length_slots == 0 {
                    self.start = index;
                }
                self.length_slots += 1;
                (kind == SlotKind::End || self.length_slots == self.count).then_some(self.start)
            }
        }
    }
}

/// The most slots one entry takes: twenty long-name pieces and the short entry.
const MOST_RUN_SLOTS: usize = 21;

/// The highest number an alias's tail carries.
const MOST_ALIAS: u32 = 999_999;

/// How many alias numbers one read of a directory counts.
pub(crate) const ALIAS_WINDOW: u32 = 1024;

/// How many windows cover every alias number from 1 to [`MOST_ALIAS`].
const ALIAS_WINDOWS: u32 = MOST_ALIAS.div_ceil(ALIAS_WINDOW);

/// The window the writer uses: [`ALIAS_WINDOW`] numbers, 64 to a word.
pub(crate) type DirectoryAliasWindow = AliasWindow<{ (ALIAS_WINDOW / 64) as usize }>;

/// **Which alias numbers a directory holds**, for the `64 * WORDS` numbers from `first` on, marked
/// as one read of the directory passes each entry. `verification.rs` proves the first free number
/// is the least one not marked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AliasWindow<const WORDS: usize> {
    first: u32,
    taken: [u64; WORDS],
}

impl<const WORDS: usize> AliasWindow<WORDS> {
    /// A window from `first` with no number taken.
    pub(crate) const fn new(first: u32) -> AliasWindow<WORDS> {
        AliasWindow {
            first,
            taken: [0; WORDS],
        }
    }

    /// The window's first number.
    #[cfg(test)]
    pub(crate) const fn first(&self) -> u32 {
        self.first
    }

    /// The window of the numbers after this one's, with none taken.
    pub(crate) const fn next(&self) -> AliasWindow<WORDS> {
        AliasWindow::new(self.first + 64 * WORDS as u32)
    }

    /// Mark `number` taken. A number outside the window changes nothing.
    pub(crate) fn mark(&mut self, number: u32) {
        let Some(offset) = number.checked_sub(self.first) else {
            return;
        };
        if let Some(word) = self.taken.get_mut((offset / 64) as usize) {
            *word |= 1 << (offset % 64);
        }
    }

    /// Mark the number of `short` taken when it is the alias of `long` numbered with its tail.
    pub(crate) fn note(&mut self, long: &LongName, short: [u8; 11]) {
        if let Some(number) = long_name::tail_number(&short)
            && long.alias_numbered(number) == Some(short)
        {
            self.mark(number);
        }
    }

    /// **The least number of the window not marked**, or `None` when every one is.
    pub(crate) fn first_free(&self) -> Option<u32> {
        let (at, word) = self
            .taken
            .iter()
            .enumerate()
            .find(|(_, word)| **word != u64::MAX)?;
        Some(self.first + at as u32 * 64 + word.trailing_ones())
    }
}

/// **The run a survey found**: its first slot, whether it reaches the end marker, and the
/// joiner as it stood at its first slot, which joins the entry written there.
#[derive(Clone, Copy)]
struct Run {
    start: u32,
    reaches_end: bool,
    joiner: Joiner,
}

/// **What a new directory entry holds**: its name and attributes, where its data starts and how
/// long it is, the entry (if any) it replaces under a rename so its own name does not collide, and
/// the times to copy from it. `None` for the times means a new entry, stamped now.
#[derive(Clone, Copy)]
struct NewEntry<'a> {
    name: &'a str,
    attributes: u8,
    first_cluster: u32,
    size_bytes: u32,
    except: Option<EntryPlace>,
    times: Option<[u8; SLOT_BYTES]>,
}

impl<D: WritableSectorDevice> Volume<D> {
    /// The root as 0 for the one place a FAT32 root may be named two ways.
    fn directory_number(&self, directory: u32) -> u32 {
        match self.boot.root_directory() {
            crate::RootDirectory::Cluster(root) if root == directory => 0,
            _ => directory,
        }
    }

    /// Make sure `directory` has a slot `upto - 1`, growing a chain directory as needed.
    fn ensure_slots(&mut self, directory: u32, upto: u32) -> Result<(), WriteError<D::Error>> {
        if upto > MOST_SLOTS {
            return Err(WriteError::DirectoryFull);
        }
        // A run is at most 21 slots, so a few clusters at the smallest size always suffice.
        for _ in 0..=upto {
            if self.slot_location(directory, upto - 1)?.is_some() {
                return Ok(());
            }
            if !self.grow_directory(directory)? {
                return Err(WriteError::DirectoryFull);
            }
        }
        Err(WriteError::DirectoryFull)
    }

    /// The first slot of a run of `count` free slots in `directory`, and whether the run reaches
    /// the directory's end marker, read in one pass.
    pub(crate) fn find_run(
        &mut self,
        directory: u32,
        count: u32,
    ) -> Result<(u32, bool), WriteError<D::Error>> {
        let run = self.survey(directory, count, None, &mut AliasWindow::new(1))?;
        Ok((run.start, run.reaches_end))
    }

    /// **Read `directory` once for a new entry of `count` slots**: the first run of free slots
    /// it fits, made sure to exist (a chain directory grows when the run reaches past its last
    /// cluster), and, for a `name`, every alias number of `name`'s basis in `window` that an entry
    /// other than `name.except` holds. Refuses with `AlreadyExists` at the first entry other
    /// than `name.except` already called `name.name`, and with `DirectoryFull` when no run fits
    /// below [`MOST_SLOTS`].
    fn survey(
        &mut self,
        directory: u32,
        count: u32,
        name: Option<(&NewEntry, Option<&LongName>)>,
        window: &mut DirectoryAliasWindow,
    ) -> Result<Run, WriteError<D::Error>> {
        let mut finder = RunFinder::new(count);
        let mut run = None;
        let mut entries = self.named_entries(DirectoryCluster(directory));
        loop {
            let Some(step) = entries.advance_slot() else {
                break;
            };
            let (_, slot, named) = step?;
            if run.is_none() {
                let kind = match slot {
                    Slot::End => SlotKind::End,
                    Slot::Free => SlotKind::Free,
                    Slot::Used(_) => SlotKind::Used,
                };
                run = finder.push(kind).map(|start| Run {
                    start,
                    reaches_end: kind == SlotKind::End,
                    joiner: entries.joiner(),
                });
                // With no name to check, the run is all the read is for.
                if run.is_some() && name.is_none() {
                    break;
                }
            }
            if let (Some(entry), Some((new, long))) = (named, name)
                && Some(entry.place()) != new.except
            {
                if entry.is_called(new.name) {
                    return Err(WriteError::AlreadyExists);
                }
                if let Some(long) = long {
                    window.note(long, entry.short_name());
                }
            }
        }
        // Past the last cluster every slot is one the directory grows, zeroed: the end marker.
        let joiner = entries.joiner();
        let run = run.unwrap_or_else(|| Run {
            start: finder.push(SlotKind::End).unwrap_or(0),
            reaches_end: true,
            joiner,
        });
        self.ensure_slots(directory, run.start + count)?;
        Ok(run)
    }

    /// The alias numbers of `long`'s basis in `window` that an entry of `directory` other than
    /// `except` holds, for a window past the one [`Volume::survey`] filled.
    fn note_aliases(
        &mut self,
        directory: u32,
        long: &LongName,
        except: Option<EntryPlace>,
        window: &mut DirectoryAliasWindow,
    ) -> Result<(), WriteError<D::Error>> {
        for entry in self.named_entries(DirectoryCluster(directory)) {
            let entry = entry?;
            if Some(entry.place()) != except {
                window.note(long, entry.short_name());
            }
        }
        Ok(())
    }

    /// Refuse `name` when it is not a legal name or an entry other than `except` already has it.
    fn check_new_name(
        &mut self,
        directory: u32,
        name: &str,
        except: Option<EntryPlace>,
    ) -> Result<(), WriteError<D::Error>> {
        if short_name_bytes(name).is_none() && LongName::new(name).is_none() {
            return Err(WriteError::NameRefused);
        }
        for entry in self.named_entries(DirectoryCluster(directory)) {
            let entry = entry?;
            if entry.is_called(name) && Some(entry.place()) != except {
                return Err(WriteError::AlreadyExists);
            }
        }
        Ok(())
    }

    /// The entry whose short slot is `short_slot` in `directory`.
    pub(crate) fn entry_at(
        &mut self,
        directory: u32,
        short_slot: u32,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        for entry in self.named_entries(DirectoryCluster(directory)) {
            let entry = entry?;
            if entry.place().short_slot == short_slot {
                return Ok(entry);
            }
        }
        Err(WriteError::EntryChanged)
    }

    /// **Write the entry `new` describes into `directory`** and return it as stored.
    ///
    /// One read of the directory checks the name, finds the run and numbers the alias, unless every
    /// alias number in the first [`ALIAS_WINDOW`] is taken, when each later window costs one read
    /// more. The slots are written a sector at a time, in directory order, the short entry's
    /// sector last, and the entry returned is joined from the slots written and the pieces the
    /// read found ahead of the run, as a later read of the directory would join it.
    fn write_entry(
        &mut self,
        directory: u32,
        new: &NewEntry,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        let directory = self.directory_number(directory);
        let short_name = short_name_bytes(new.name);
        let long = match short_name {
            Some(_) => None,
            None => Some(LongName::new(new.name).ok_or(WriteError::NameRefused)?),
        };
        let pieces = long.as_ref().map_or(0, LongName::entries) as u32;
        let mut window = AliasWindow::new(1);
        let run = self.survey(
            directory,
            pieces + 1,
            Some((new, long.as_ref())),
            &mut window,
        )?;
        // No other entry holds the plain basis: one that did would be called `new.name`, which
        // the survey refuses, so the basis needs no check of its own.
        let short = match (&long, short_name, long.as_ref().and_then(LongName::basis)) {
            (_, Some(short), _) => short,
            (Some(_), None, Some(basis)) => basis,
            (Some(long), None, None) => self.unused_alias(directory, long, new.except, window)?,
            (None, None, _) => return Err(WriteError::NameRefused),
        };
        self.begin_change()?;
        let mut slots = [[0u8; SLOT_BYTES]; MOST_RUN_SLOTS];
        let sum = long_name::checksum(&short);
        if let Some(long) = &long {
            for (piece, slot) in slots.iter_mut().enumerate().take(pieces as usize) {
                long.entry(piece, sum, slot);
            }
        }
        let slots = &mut slots[..=pieces as usize];
        let slot = &mut slots[pieces as usize];
        slot[..11].copy_from_slice(&short);
        slot[11] = new.attributes;
        if self.boot.file_allocation_table_type() == FileAllocationTableType::Fat32 {
            slot[20..22].copy_from_slice(&((new.first_cluster >> 16) as u16).to_le_bytes());
        }
        slot[26..28].copy_from_slice(&(new.first_cluster as u16).to_le_bytes());
        slot[28..32].copy_from_slice(&new.size_bytes.to_le_bytes());
        // A new entry is created, written and accessed now; a moved one keeps its times.
        match new.times {
            None => self.stamp.write_new(slot),
            Some(old) => crate::time::copy_times(&old, slot),
        }
        self.write_run(directory, run.start, slots, run.reaches_end)?;
        let file_allocation_table_type = self.boot.file_allocation_table_type();
        let mut joiner = run.joiner;
        let mut written = None;
        for (index, slot) in (run.start..).zip(slots.iter()) {
            if let Slot::Used(entry) = Slot::parse(slot, file_allocation_table_type) {
                written = joiner.take(directory, index, entry);
            }
        }
        written.ok_or(WriteError::EntryChanged)
    }

    /// **Write `slots` to `directory` from slot `start` on**, a sector at a time and in order, so
    /// the last slot's sector goes down last. With `reaches_end`, the slot after the last is made
    /// an end marker when it holds anything else, since the slots after an end marker may hold
    /// stale bytes.
    fn write_run(
        &mut self,
        directory: u32,
        start: u32,
        slots: &[[u8; SLOT_BYTES]],
        reaches_end: bool,
    ) -> Result<(), WriteError<D::Error>> {
        let size = self.boot.sector_size().bytes();
        let mut buffer = [0u8; SectorSize::MAXIMUM_BYTES];
        let mut open = None;
        for (index, slot) in (start..).zip(slots) {
            let Some((sector, offset)) = self.slot_location(directory, index)? else {
                break;
            };
            if open != Some(sector) {
                if let Some(full) = open {
                    self.write_data(full, &buffer[..size])?;
                }
                self.read_sector(sector, &mut buffer[..size])?;
                open = Some(sector);
            }
            buffer[offset..offset + SLOT_BYTES].copy_from_slice(slot);
        }
        let after = start + slots.len() as u32;
        let end = match reaches_end {
            true => self.slot_location(directory, after)?,
            false => None,
        };
        match (open, end) {
            (Some(full), Some((sector, offset))) if full == sector => {
                buffer[offset] = 0;
                self.write_data(full, &buffer[..size])?;
            }
            (open, end) => {
                if let Some(full) = open {
                    self.write_data(full, &buffer[..size])?;
                }
                if let Some((sector, offset)) = end {
                    self.read_sector(sector, &mut buffer[..size])?;
                    if buffer[offset] != 0 {
                        buffer[offset] = 0;
                        self.write_data(sector, &buffer[..size])?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The first alias of `long`, numbered from 1, that no entry of `directory` other than
    /// `except` holds. `window` is the first window, which the survey filled.
    fn unused_alias(
        &mut self,
        directory: u32,
        long: &LongName,
        except: Option<EntryPlace>,
        mut window: DirectoryAliasWindow,
    ) -> Result<[u8; 11], WriteError<D::Error>> {
        for _ in 0..ALIAS_WINDOWS {
            if let Some(number) = window.first_free() {
                return match long.alias_numbered(number) {
                    Some(alias) => Ok(alias),
                    None => Err(WriteError::DirectoryFull),
                };
            }
            window = window.next();
            self.note_aliases(directory, long, except, &mut window)?;
        }
        Err(WriteError::DirectoryFull)
    }

    /// **Create an empty file called `name` in the directory whose first cluster is `directory`**
    /// (0 for the root), and return its entry.
    ///
    /// A name that is a legal upper-case 8.3 name is written as a short entry alone. Any other
    /// legal name (up to 255 Universal Character Set (UCS) 2 units, mixed case, spaces inside,
    /// several dots) gets long-name slots ahead of a short alias numbered against the directory.
    /// The entry has the archive attribute, no clusters, size 0 and zero timestamps. A full
    /// directory grows by a cluster unless it is the fixed root of a FAT12 or FAT16 volume.
    pub(crate) fn create_file(
        &mut self,
        directory: u32,
        name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        self.write_entry(
            directory,
            &NewEntry {
                name,
                attributes: ARCHIVE,
                first_cluster: 0,
                size_bytes: 0,
                except: None,
                times: None,
            },
        )
    }

    /// **Create an empty directory called `name`** in `directory` (0 for the root), and return its
    /// entry. The new directory holds `.` and `..` in a cluster of its own, zeroed past them.
    pub(crate) fn create_directory(
        &mut self,
        directory: u32,
        name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        let parent = self.directory_number(directory);
        self.check_new_name(parent, name, None)?;
        self.begin_change()?;
        let cluster = self.append_cluster()?.ok_or(WriteError::VolumeFull)?;
        let made = self.write_dot_entries(cluster, parent).and_then(|()| {
            self.write_entry(
                parent,
                &NewEntry {
                    name,
                    attributes: DIRECTORY,
                    first_cluster: cluster,
                    size_bytes: 0,
                    except: None,
                    times: None,
                },
            )
        });
        if made.is_err() {
            let _ = self.deallocate_chain(cluster);
        }
        made
    }

    /// Take one free cluster as the head of a new chain.
    fn append_cluster(&mut self) -> Result<Option<u32>, WriteError<D::Error>> {
        Ok(crate::allocation::LinksMut::append(self, None)?)
    }

    /// Fill `cluster` with `.` and `..` (whose cluster is `parent`, 0 for the root) and zeros.
    fn write_dot_entries(&mut self, cluster: u32, parent: u32) -> Result<(), WriteError<D::Error>> {
        let boot = self.boot;
        let first = boot
            .cluster_first_sector(cluster)
            .ok_or(ChainError::NotADataCluster(cluster))?;
        let size = boot.sector_size().bytes();
        let mut sector = [0u8; SectorSize::MAXIMUM_BYTES];
        for (at, (name, target)) in [(b".          ", cluster), (b"..         ", parent)]
            .iter()
            .enumerate()
        {
            let slot = &mut sector[at * SLOT_BYTES..(at + 1) * SLOT_BYTES];
            slot[..11].copy_from_slice(*name);
            slot[11] = DIRECTORY;
            if boot.file_allocation_table_type() == FileAllocationTableType::Fat32 {
                slot[20..22].copy_from_slice(&((*target >> 16) as u16).to_le_bytes());
            }
            slot[26..28].copy_from_slice(&(*target as u16).to_le_bytes());
            self.stamp.write_new(slot);
        }
        Medium::write_data(self, first, &sector[..size])?;
        let zeros = [0u8; SectorSize::MAXIMUM_BYTES];
        for later in 1..boot.sectors_per_cluster() {
            Medium::write_data(self, first + later, &zeros[..size])?;
        }
        Ok(())
    }

    /// Check the slot `entry` names still holds it, or `EntryChanged`. See
    /// [`Volume::slot_holds`].
    pub(crate) fn verify(&mut self, entry: &NamedEntry) -> Result<(), WriteError<D::Error>> {
        if self.slot_holds(entry.place(), entry.short_entry())? {
            Ok(())
        } else {
            Err(WriteError::EntryChanged)
        }
    }

    /// Mark every slot of `entry`, its long-name run and its short entry, free.
    fn free_slots(&mut self, entry: &NamedEntry) -> Result<(), WriteError<D::Error>> {
        let place = entry.place();
        for index in place.first_slot..=place.short_slot {
            let mut slot = self
                .read_slot(place.directory, index)?
                .ok_or(WriteError::EntryChanged)?;
            slot[0] = FREE;
            self.write_slot(place.directory, index, &slot)?;
        }
        Ok(())
    }

    /// **Remove the empty directory `entry` names**: mark its slots free, then free its cluster.
    /// Refuses a file, `.` and `..`, and a directory that holds anything else.
    pub(crate) fn remove_directory(
        &mut self,
        entry: &NamedEntry,
    ) -> Result<(), WriteError<D::Error>> {
        if !entry.is_directory() || entry.is_dot_entry() {
            return Err(WriteError::NotADirectory);
        }
        self.verify(entry)?;
        for inner in self.named_entries(DirectoryCluster(entry.first_cluster())) {
            if !inner?.is_dot_entry() {
                return Err(WriteError::DirectoryNotEmpty);
            }
        }
        self.begin_change()?;
        self.free_slots(entry)?;
        self.deallocate_chain(entry.first_cluster())
            .map_err(|_| WriteError::EntryChanged)?;
        Ok(())
    }

    /// Whether `ancestor` is `directory` or one of the directories above it, by `..` links.
    fn is_inside(&mut self, ancestor: u32, directory: u32) -> Result<bool, WriteError<D::Error>> {
        let mut current = self.directory_number(directory);
        for _ in 0..=self.boot.clusters() {
            if current == ancestor {
                return Ok(true);
            }
            if current == 0 {
                return Ok(false);
            }
            let Some(dotdot) = self.read_slot(current, 1)? else {
                return Ok(false);
            };
            let high = if self.boot.file_allocation_table_type() == FileAllocationTableType::Fat32 {
                u32::from(u16::from_le_bytes([dotdot[20], dotdot[21]]))
            } else {
                0
            };
            current = self.directory_number(
                (high << 16) | u32::from(u16::from_le_bytes([dotdot[26], dotdot[27]])),
            );
        }
        Err(WriteError::Chain(ChainError::TooLong))
    }

    /// **Rename `entry`, and move it to `directory`** (0 for the root; its own directory to rename
    /// in place), and return the entry as it now stands. The data does not move: a new entry
    /// points at the same clusters, then the old one is freed. A directory moved to another parent
    /// has its `..` updated. Refuses a name another entry holds, a file or directory moved into
    /// itself or below itself, and `.` and `..`.
    pub(crate) fn rename(
        &mut self,
        entry: &NamedEntry,
        directory: u32,
        new_name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        if entry.is_dot_entry() || entry.is_volume_label() {
            return Err(WriteError::NotAFile);
        }
        self.verify(entry)?;
        let target = self.directory_number(directory);
        if entry.is_directory() && self.is_inside(entry.first_cluster(), target)? {
            return Err(WriteError::MovedIntoItself);
        }
        let old_slot = self
            .read_slot(entry.place().directory, entry.place().short_slot)?
            .ok_or(WriteError::EntryChanged)?;
        let moved = self.write_entry(
            target,
            &NewEntry {
                name: new_name,
                attributes: entry.attributes(),
                first_cluster: entry.first_cluster(),
                size_bytes: entry.size_bytes(),
                except: Some(entry.place()),
                times: Some(old_slot),
            },
        )?;
        if entry.is_directory() && entry.place().directory != target {
            let first = entry.first_cluster();
            let mut dotdot = self.read_slot(first, 1)?.ok_or(WriteError::EntryChanged)?;
            if self.boot.file_allocation_table_type() == FileAllocationTableType::Fat32 {
                dotdot[20..22].copy_from_slice(&((target >> 16) as u16).to_le_bytes());
            }
            dotdot[26..28].copy_from_slice(&(target as u16).to_le_bytes());
            self.write_slot(first, 1, &dotdot)?;
        }
        self.free_slots(entry)?;
        Ok(moved)
    }
}

/// Keeps the lookup error convertible where the directory tree reports one.
impl<E> From<LookupError<E>> for WriteError<E> {
    fn from(error: LookupError<E>) -> Self {
        match error {
            LookupError::Chain(error) => WriteError::Chain(error),
            LookupError::NotFound | LookupError::RootHasNoEntry => WriteError::EntryChanged,
            LookupError::NotADirectory => WriteError::NotADirectory,
        }
    }
}

#[cfg(kani)]
mod verification;

#[cfg(test)]
mod tests;
