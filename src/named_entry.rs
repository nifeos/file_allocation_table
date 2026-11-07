//! **A directory read as files**: each short entry with the long name its pieces spell, checked
//! against the entry's checksum.
//!
//! [`Directory`] reports slots as stored. [`NamedEntries`] wraps it and joins every run of long-name
//! pieces to the short entry after it, through [`LongNameAssembler`]. A run the assembler refuses
//! does not stop the listing: the short entry is still yielded, with no long name and the reason
//! the run was refused, because a stranger's volume may hold a stale piece and its files remain
//! reachable by their short names.

use crate::directory::{Directory, DirectoryEntry, ShortEntry, Slot};
use crate::long_name_assembly::{LongFileName, LongFileNameError, LongNameAssembler};
use crate::reader::ChainError;
use crate::sector::SectorDevice;
use crate::time::CivilDateTime;

/// **A directory entry with its names**: the short entry, and the long name when the entry has one
/// that passed its checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NamedEntry {
    short: ShortEntry,
    long_name: Result<Option<LongFileName>, LongFileNameError>,
    place: EntryPlace,
}

/// **Where an entry sits**: its directory's first cluster (0 for the root), the index of the
/// first slot of its long-name run, and the index of its short slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntryPlace {
    pub(crate) directory: u32,
    pub(crate) first_slot: u32,
    pub(crate) short_slot: u32,
}

impl NamedEntry {
    /// An entry at `place` with no long name, for the writer's tests.
    #[cfg(test)]
    pub(crate) const fn from_parts(short: ShortEntry, place: EntryPlace) -> NamedEntry {
        NamedEntry {
            short,
            long_name: Ok(None),
            place,
        }
    }

    /// Where the entry sits in its directory.
    pub(crate) const fn place(&self) -> EntryPlace {
        self.place
    }

    /// The short entry as stored.
    pub const fn short_entry(&self) -> &ShortEntry {
        &self.short
    }

    /// The eleven short-name bytes as stored: eight of base and three of extension, space padded.
    pub const fn short_name(&self) -> [u8; 11] {
        self.short.name()
    }

    /// The long name, when the entry has one and its run was whole, ordered and bound to this
    /// entry by its checksum.
    pub const fn long_name(&self) -> Option<&LongFileName> {
        match &self.long_name {
            Ok(Some(name)) => Some(name),
            _ => None,
        }
    }

    /// Why the long-name pieces ahead of this entry were refused, or `None` when there were none
    /// or they were accepted.
    pub const fn long_name_refusal(&self) -> Option<LongFileNameError> {
        match &self.long_name {
            Err(error) => Some(*error),
            Ok(_) => None,
        }
    }

    /// `DIR_Attr`, every bit as stored. [`NamedEntry::user_attributes`] is the four bits a caller
    /// may set, as a set.
    pub const fn attributes(&self) -> u8 {
        self.short.attributes()
    }

    /// The first cluster of the entry's chain, or 0 for an empty file. Not checked against the
    /// volume.
    pub const fn first_cluster(&self) -> u32 {
        self.short.first_cluster()
    }

    /// The file's length in bytes; 0 for a directory.
    pub const fn size_bytes(&self) -> u32 {
        self.short.size_bytes()
    }

    /// When the entry was created; see [`ShortEntry::created`].
    pub const fn created(&self) -> Option<CivilDateTime> {
        self.short.created()
    }

    /// When the entry was last written; see [`ShortEntry::modified`].
    pub const fn modified(&self) -> Option<CivilDateTime> {
        self.short.modified()
    }

    /// The day the entry was last accessed; see [`ShortEntry::accessed`].
    pub const fn accessed(&self) -> Option<CivilDateTime> {
        self.short.accessed()
    }

    /// Whether the entry is a directory.
    pub const fn is_directory(&self) -> bool {
        self.short.is_directory()
    }

    /// Whether the entry is the volume label, which names no file.
    pub const fn is_volume_label(&self) -> bool {
        self.short.is_volume_label()
    }

    /// Whether the entry is `.` or `..`, the links a subdirectory begins with.
    pub fn is_dot_entry(&self) -> bool {
        let name = self.short.name();
        self.short.is_directory() && (name == *b".          " || name == *b"..         ")
    }
}

/// **A directory, read as named entries**: what [`Volume::root_named_entries`] and [`Volume::named_entries`]
/// return.
///
/// It yields every used short entry in order, `.`, `..` and the volume label included, each with
/// its long name. It ends where [`Directory`] ends, and after an error it yields nothing more.
///
/// [`Volume::root_named_entries`]: crate::Volume::root_named_entries
/// [`Volume::named_entries`]: crate::Volume::named_entries
#[derive(Debug)]
pub struct NamedEntries<'v, D> {
    directory: Directory<'v, D>,
    joiner: Joiner,
}

/// **Joins a directory's used slots into named entries**, one slot at a time: the long-name pieces
/// since the last short entry, and where the first of them sits. [`NamedEntries`] feeds it every
/// used slot; the writer copies it where it lays down a new entry, so it can name the entry it
/// wrote without reading the directory again.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Joiner {
    assembler: LongNameAssembler,
    /// The index of the first long-name piece since the last short entry.
    run_start: Option<u32>,
}

impl Joiner {
    /// A joiner at a directory's start.
    pub(crate) const fn new() -> Joiner {
        Joiner {
            assembler: LongNameAssembler::new(),
            run_start: None,
        }
    }

    /// **Take the used slot at `index` of `directory`**, and return the named entry a short slot
    /// completes. A long-name piece joins the open run and returns `None`.
    pub(crate) fn take(
        &mut self,
        directory: u32,
        index: u32,
        entry: DirectoryEntry,
    ) -> Option<NamedEntry> {
        match entry {
            DirectoryEntry::LongName(piece) => {
                self.run_start.get_or_insert(index);
                self.assembler.push(&piece);
                None
            }
            DirectoryEntry::Short(short) => {
                let long_name = self.assembler.finish(&short.name());
                let place = EntryPlace {
                    directory,
                    first_slot: self.run_start.take().unwrap_or(index),
                    short_slot: index,
                };
                Some(NamedEntry {
                    short,
                    long_name,
                    place,
                })
            }
        }
    }
}

impl<'v, D> NamedEntries<'v, D> {
    /// Read `directory` as named entries.
    pub(crate) const fn new(directory: Directory<'v, D>) -> NamedEntries<'v, D> {
        NamedEntries {
            directory,
            joiner: Joiner::new(),
        }
    }

    /// The joiner as it stands after the slots read so far.
    pub(crate) const fn joiner(&self) -> Joiner {
        self.joiner
    }
}

/// A slot's index, the slot, and the named entry a short slot completes, or the error that ended
/// the read.
pub(crate) type JoinedSlot<E> = Result<(u32, Slot, Option<NamedEntry>), ChainError<E>>;

impl<D: SectorDevice> NamedEntries<'_, D> {
    /// **The next slot, free and end slots included**, with the named entry a short slot
    /// completes. It ends as [`Directory::advance_slot`] ends.
    pub(crate) fn advance_slot(&mut self) -> Option<JoinedSlot<D::Error>> {
        let (index, slot) = match self.directory.advance_slot()? {
            Ok(step) => step,
            Err(error) => return Some(Err(error)),
        };
        let named = match slot {
            Slot::Used(entry) => self
                .joiner
                .take(self.directory.first_cluster(), index, entry),
            Slot::Free | Slot::End => None,
        };
        Some(Ok((index, slot, named)))
    }
}

impl<D: SectorDevice> Iterator for NamedEntries<'_, D> {
    type Item = Result<NamedEntry, ChainError<D::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.directory.advance_indexed()? {
                Err(error) => return Some(Err(error)),
                Ok((index, entry)) => {
                    let directory = self.directory.first_cluster();
                    if let Some(named) = self.joiner.take(directory, index, entry) {
                        return Some(Ok(named));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
