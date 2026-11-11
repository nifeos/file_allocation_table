//! Run placement, alias numbering, directories and renames on the small FAT12 volume
//! `reader::tests` builds.

use super::*;
use crate::reader::tests::Image;
use crate::{DirectoryCluster, NoClock, WritableVolume};

fn feed(count: u32, kinds: &[SlotKind]) -> Option<u32> {
    let mut finder = RunFinder::new(count);
    kinds.iter().find_map(|kind| finder.push(*kind))
}

#[test]
fn a_run_starts_at_the_first_gap_that_is_long_enough_and_an_end_marker_completes_it() {
    use SlotKind::{End, Free, Used};
    assert_eq!(feed(1, &[Used, Free]), Some(1));
    assert_eq!(feed(2, &[Free, Used, Free, Free]), Some(2));
    assert_eq!(feed(3, &[Free, Free, Used, Free, Free]), None);
    assert_eq!(feed(3, &[Used, Free, Free, End]), Some(1));
    assert_eq!(feed(4, &[Used, End]), Some(1));
    assert_eq!(feed(2, &[End]), Some(0));
}

#[test]
fn aliases_count_up_and_give_the_base_room_for_the_tail() {
    let name = LongName::new("A Long Name Number 7.txt").unwrap();
    assert_eq!(name.alias_numbered(1), Some(*b"ALONGN~1TXT"));
    assert_eq!(name.alias_numbered(9), Some(*b"ALONGN~9TXT"));
    assert_eq!(name.alias_numbered(10), Some(*b"ALONG~10TXT"));
    assert_eq!(name.alias_numbered(99), Some(*b"ALONG~99TXT"));
    assert_eq!(name.alias_numbered(100), Some(*b"ALON~100TXT"));
    assert_eq!(name.alias_numbered(999_999), Some(*b"A~999999TXT"));
    assert_eq!(name.alias_numbered(0), None);
    assert_eq!(name.alias_numbered(1_000_000), None);
}

#[test]
fn a_window_marks_only_its_own_numbers_and_finds_the_least_free_one() {
    let mut window = AliasWindow::<2>::new(65);
    assert_eq!(window.first(), 65);
    assert_eq!(window.first_free(), Some(65));
    // Below and past the window's 128 numbers.
    window.mark(64);
    window.mark(193);
    window.mark(0);
    assert_eq!(window.first_free(), Some(65));
    for number in 65..135 {
        window.mark(number);
    }
    assert_eq!(window.first_free(), Some(135));
    window.mark(136);
    assert_eq!(window.first_free(), Some(135));
    for number in 135..193 {
        window.mark(number);
    }
    assert_eq!(window.first_free(), None);
    let next = window.next();
    assert_eq!(next.first(), 193);
    assert_eq!(next.first_free(), Some(193));
    assert_eq!(
        DirectoryAliasWindow::new(1).next().first(),
        1 + ALIAS_WINDOW
    );
}

#[test]
fn a_window_counts_an_alias_of_its_own_basis_and_number_only() {
    let long = LongName::new("Long File One.txt").unwrap();
    let mut window = DirectoryAliasWindow::new(1);
    window.note(&long, *b"LONGFI~1TXT");
    // Another basis, another extension, a leading zero the writer never makes, and no tail.
    for other in [
        b"OTHERS~2TXT",
        b"LONGFI~2TX ",
        b"LONGF~02TXT",
        b"LONGFILETXT",
    ] {
        window.note(&long, *other);
    }
    assert_eq!(window.first_free(), Some(2));
    window.note(&long, *b"LONGFI~2TXT");
    window.note(&long, *b"LONGF~10TXT");
    assert_eq!(window.first_free(), Some(3));
}

/// The 8.3 name a short entry's eleven bytes spell, dot and all.
fn spelled(short: [u8; 11]) -> String {
    let base = core::str::from_utf8(&short[..8]).unwrap().trim_end();
    let extension = core::str::from_utf8(&short[8..]).unwrap().trim_end();
    format!("{base}.{extension}")
}

/// A directory whose first [`ALIAS_WINDOW`] alias numbers are all held, by short names, makes the
/// writer read it again for the next window; a number freed in the first window is taken first.
#[test]
#[cfg_attr(miri, ignore = "creates over a thousand entries in one directory")]
fn an_alias_past_the_first_window_is_the_next_free_number() {
    use crate::{FormatOptions, FormatPlan, SectorSize};
    let sectors = 16 * 1024u32;
    let plan = FormatPlan::new(
        FormatOptions::new(SectorSize::Bytes512, sectors),
        &mut NoClock,
    )
    .unwrap();
    let mut device = Blank(vec![0; sectors as usize * 512]);
    plan.write_to(&mut device).unwrap();
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "MANY")
        .unwrap()
        .first_cluster();
    let long = LongName::new("Long File.txt").unwrap();
    let mut seventh = None;
    for number in 1..=ALIAS_WINDOW + 6 {
        let alias = long.alias_numbered(number).unwrap();
        let entry = volume
            .create_file(DirectoryCluster(directory), &spelled(alias))
            .unwrap();
        assert_eq!(entry.short_name(), alias);
        if number == 7 {
            seventh = Some(entry);
        }
    }
    let entry = volume
        .create_file(DirectoryCluster(directory), "Long File.txt")
        .unwrap();
    assert_eq!(entry.short_name(), *b"LON~1031TXT");
    // A rename past the first window may keep its own number there.
    let entry = volume
        .rename(&entry, DirectoryCluster(directory), "Long File Renamed.txt")
        .unwrap();
    assert_eq!(entry.short_name(), *b"LON~1031TXT");
    volume.remove_file(&seventh.unwrap()).unwrap();
    let entry = volume
        .create_file(DirectoryCluster(directory), "Long File 2.txt")
        .unwrap();
    assert_eq!(entry.short_name(), *b"LONGFI~7TXT");
}

#[test]
fn a_rename_may_keep_its_own_alias() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let one = volume
        .create_file(DirectoryCluster::ROOT, "Long File One.txt")
        .unwrap();
    assert_eq!(one.short_name(), *b"LONGFI~1TXT");
    let renamed = volume
        .rename(&one, DirectoryCluster::ROOT, "Long File Two.txt")
        .unwrap();
    assert_eq!(renamed.short_name(), *b"LONGFI~1TXT");
    assert!(volume.lookup("Long File One.txt").is_err());
}

/// The entry a create returns is the one a read of the directory joins, even when a stray
/// long-name piece sits ahead of the run: the read counts it into the run's slots.
#[test]
fn the_entry_a_create_returns_is_the_one_a_read_joins() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    volume.create_file(DirectoryCluster::ROOT, "A.TXT").unwrap();
    let mut stray = [0xFFu8; SLOT_BYTES];
    LongName::new("stray").unwrap().entry(0, 0x5A, &mut stray);
    volume.write_slot(0, 1, &stray).unwrap();
    for name in ["A long name after a stray piece.txt", "B.TXT"] {
        let made = volume.create_file(DirectoryCluster::ROOT, name).unwrap();
        let read = volume.entry_at(0, made.place().short_slot).unwrap();
        assert_eq!(made, read);
    }
    let made = volume
        .lookup("A long name after a stray piece.txt")
        .unwrap();
    assert_eq!(made.place().first_slot, 1);
    assert!(made.long_name().is_some());
}

/// A cluster of one sector holds sixteen slots: a stale slot after the end marker, in the sector
/// the new entry goes into or in the next one, becomes an end marker.
#[test]
fn a_stale_slot_after_the_end_marker_is_ended_in_the_same_sector_or_the_next() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "SUB")
        .unwrap()
        .first_cluster();
    let mut stale = [0u8; SLOT_BYTES];
    stale[..11].copy_from_slice(b"STALE   TXT");
    stale[11] = ARCHIVE;
    // Slot 3 holds stale bytes after the end marker at 2, in the cluster's one sector.
    volume.write_slot(directory, 3, &stale).unwrap();
    volume
        .create_file(DirectoryCluster(directory), "B.TXT")
        .unwrap();
    assert_eq!(volume.read_slot(directory, 3).unwrap().unwrap()[0], 0);
    assert_eq!(
        volume.read_slot(directory, 3).unwrap().unwrap()[1..],
        stale[1..]
    );
    // Fill the sector to slot 14, then put stale bytes at 16, in the next cluster's sector.
    for name in "CDEFGHIJKLMN".chars() {
        volume
            .create_file(DirectoryCluster(directory), &format!("{name}.TXT"))
            .unwrap();
    }
    assert!(volume.grow_directory(directory).unwrap());
    volume.write_slot(directory, 16, &stale).unwrap();
    let last = volume
        .create_file(DirectoryCluster(directory), "O.TXT")
        .unwrap();
    assert_eq!(last.place().short_slot, 15);
    assert_eq!(volume.read_slot(directory, 16).unwrap().unwrap()[0], 0);
    let names = volume
        .named_entries(DirectoryCluster(directory))
        .map(|entry| entry.unwrap().short_name())
        .filter(|name| name[0] != b'.')
        .count();
    assert_eq!(names, 14);
}

#[test]
fn colliding_long_names_take_successive_aliases_and_a_freed_alias_is_reused() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let first = volume
        .create_file(DirectoryCluster::ROOT, "Long File One.txt")
        .unwrap();
    let second = volume
        .create_file(DirectoryCluster::ROOT, "Long File Two.txt")
        .unwrap();
    assert_eq!(first.short_name(), *b"LONGFI~1TXT");
    assert_eq!(second.short_name(), *b"LONGFI~2TXT");
    volume.remove_file(&first).unwrap();
    let third = volume
        .create_file(DirectoryCluster::ROOT, "Long File Three.txt")
        .unwrap();
    assert_eq!(third.short_name(), *b"LONGFI~1TXT");
    assert!(volume.lookup("long file three.TXT").is_ok());
    assert_eq!(
        volume.create_file(DirectoryCluster::ROOT, "LONG FILE TWO.txt"),
        Err(WriteError::AlreadyExists)
    );
}

#[test]
fn a_long_run_fits_a_gap_of_deleted_slots_and_not_a_shorter_one() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let a = volume.create_file(DirectoryCluster::ROOT, "A.TXT").unwrap();
    volume.create_file(DirectoryCluster::ROOT, "B.TXT").unwrap();
    let c = volume.create_file(DirectoryCluster::ROOT, "C.TXT").unwrap();
    volume.create_file(DirectoryCluster::ROOT, "D.TXT").unwrap();
    volume.remove_file(&a).unwrap();
    volume.remove_file(&c).unwrap();
    // One slot gaps at 0 and 2; a two-piece name needs three slots, so it goes after D.
    let long = volume
        .create_file(
            DirectoryCluster::ROOT,
            "A name that needs two long pieces.txt",
        )
        .unwrap();
    assert_eq!(long.place().first_slot, 4);
    assert_eq!(long.place().short_slot, 7);
    // A short name takes the first gap.
    assert_eq!(
        volume
            .create_file(DirectoryCluster::ROOT, "E.TXT")
            .unwrap()
            .place()
            .short_slot,
        0
    );
}

#[test]
fn a_directory_holds_dot_entries_and_is_removed_only_when_empty() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let free = volume.count_free_clusters().unwrap();
    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "Sub Directory")
        .unwrap();
    assert!(directory.is_directory());
    assert_eq!(volume.count_free_clusters().unwrap(), free - 1);
    let names: usize = volume
        .named_entries(directory.directory_cluster().unwrap())
        .map(|e| e.unwrap())
        .filter(|e| e.is_dot_entry())
        .count();
    assert_eq!(names, 2);
    let inner = volume
        .create_file(directory.directory_cluster().unwrap(), "inner.txt")
        .unwrap();
    assert_eq!(
        volume.remove_directory(&directory),
        Err(WriteError::DirectoryNotEmpty)
    );
    volume.remove_file(&inner).unwrap();
    volume.remove_directory(&directory).unwrap();
    assert_eq!(volume.count_free_clusters().unwrap(), free);
    assert!(volume.lookup("Sub Directory").is_err());
    let file = volume.create_file(DirectoryCluster::ROOT, "F.TXT").unwrap();
    assert_eq!(
        volume.remove_directory(&file),
        Err(WriteError::NotADirectory)
    );
}

#[test]
fn a_failed_create_directory_gives_its_cluster_back() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    volume
        .create_directory(DirectoryCluster::ROOT, "D")
        .unwrap();
    let free = volume.count_free_clusters().unwrap();
    assert_eq!(
        volume.create_directory(DirectoryCluster::ROOT, "d"),
        Err(WriteError::AlreadyExists)
    );
    assert_eq!(
        volume.create_directory(DirectoryCluster::ROOT, "bad*name"),
        Err(WriteError::NameRefused)
    );
    assert_eq!(volume.count_free_clusters().unwrap(), free);
}

#[test]
fn a_rename_keeps_the_data_and_a_moved_directory_gets_a_new_dotdot() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let a = volume
        .create_directory(DirectoryCluster::ROOT, "A")
        .unwrap();
    let b = volume
        .create_directory(DirectoryCluster::ROOT, "B")
        .unwrap();
    let file = volume
        .create_file(a.directory_cluster().unwrap(), "data.bin")
        .unwrap();
    volume
        .open_file_writer(&file)
        .unwrap()
        .write(&[7; 600])
        .unwrap();
    let file = volume.lookup("A/data.bin").unwrap();
    let moved = volume
        .rename(&file, b.directory_cluster().unwrap(), "Data Renamed.bin")
        .unwrap();
    assert_eq!(
        (moved.first_cluster(), moved.size_bytes()),
        (file.first_cluster(), 600)
    );
    assert!(volume.lookup("A/data.bin").is_err());
    assert!(volume.lookup("B/data renamed.BIN").is_ok());
    // The moved file's bytes are untouched (a move rewrites entries, never clusters).
    let entry = volume.lookup("B/data renamed.BIN").unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    let mut out = [0u8; 700];
    assert_eq!(reader.read(&mut out).unwrap(), 600);
    assert!(out[..600].iter().all(|b| *b == 7));
    let a = volume.lookup("A").unwrap();
    volume
        .rename(&a, b.directory_cluster().unwrap(), "A Inside B")
        .unwrap();
    let dotdot = volume.read_slot(a.first_cluster(), 1).unwrap().unwrap();
    assert_eq!(
        u16::from_le_bytes([dotdot[26], dotdot[27]]) as u32,
        b.first_cluster()
    );
    assert!(volume.lookup("B/A Inside B").is_ok());
    // And back to the root, where `..` is 0.
    let inside = volume.lookup("B/A Inside B").unwrap();
    volume
        .rename(&inside, DirectoryCluster::ROOT, "A Again")
        .unwrap();
    let dotdot = volume.read_slot(a.first_cluster(), 1).unwrap().unwrap();
    assert_eq!(u16::from_le_bytes([dotdot[26], dotdot[27]]), 0);
}

#[test]
fn a_directory_cannot_move_into_itself_or_below_and_dots_cannot_be_renamed() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let a = volume
        .create_directory(DirectoryCluster::ROOT, "A")
        .unwrap();
    let inner = volume
        .create_directory(a.directory_cluster().unwrap(), "Inner")
        .unwrap();
    let innermost = volume
        .create_directory(inner.directory_cluster().unwrap(), "Innermost")
        .unwrap();
    for target in [
        a.first_cluster(),
        inner.first_cluster(),
        innermost.first_cluster(),
    ] {
        assert_eq!(
            volume.rename(&a, DirectoryCluster(target), "X"),
            Err(WriteError::MovedIntoItself)
        );
    }
    assert!(volume.rename(&inner, DirectoryCluster::ROOT, "Out").is_ok());
    let dot = volume
        .named_entries(a.directory_cluster().unwrap())
        .map(|e| e.unwrap())
        .find(|e| e.is_dot_entry())
        .unwrap();
    assert_eq!(
        volume.rename(&dot, DirectoryCluster::ROOT, "Dot"),
        Err(WriteError::NotAFile)
    );
    assert_eq!(
        volume.remove_directory(&dot),
        Err(WriteError::NotADirectory)
    );
}

#[test]
fn a_rename_that_only_changes_case_keeps_one_entry() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let file = volume
        .create_file(DirectoryCluster::ROOT, "Mixed Case Name.txt")
        .unwrap();
    let again = volume
        .rename(&file, DirectoryCluster::ROOT, "MIXED case NAME.txt")
        .unwrap();
    assert_ne!(again.place().short_slot, file.place().short_slot);
    let listed = volume
        .root_named_entries()
        .map(|e| e.unwrap())
        .filter(|e| !e.is_volume_label())
        .count();
    assert_eq!(listed, 1);
}

#[test]
fn a_stale_entry_is_refused_by_a_directory_remove_and_a_rename() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "D")
        .unwrap();
    let file = volume.create_file(DirectoryCluster::ROOT, "F.TXT").unwrap();
    // The directory's slot becomes a long-name piece; the file's slot is freed and reused.
    let slot = directory.place().short_slot as usize;
    volume.remove_file(&file).unwrap();
    assert_eq!(
        volume.rename(&file, DirectoryCluster::ROOT, "G.TXT"),
        Err(WriteError::EntryChanged)
    );
    let mut image = volume.into_device();
    image.bytes[5 * 512 + slot * 32 + 11] = 0x0F;
    let mut volume = WritableVolume::open(image, NoClock).unwrap();
    assert_eq!(
        volume.remove_directory(&directory),
        Err(WriteError::EntryChanged)
    );
    assert_eq!(
        volume.rename(&directory, DirectoryCluster::ROOT, "E"),
        Err(WriteError::EntryChanged)
    );
}

#[test]
fn a_volume_label_cannot_be_renamed() {
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    let file = volume.create_file(DirectoryCluster::ROOT, "L.TXT").unwrap();
    let mut image = volume.into_device();
    image.bytes[5 * 512 + 11] = 0x08;
    let mut volume = WritableVolume::open(image, NoClock).unwrap();
    let label = volume
        .root_named_entries()
        .map(|e| e.unwrap())
        .next()
        .unwrap();
    assert!(label.is_volume_label());
    assert_eq!(
        volume.rename(&label, DirectoryCluster::ROOT, "NEW"),
        Err(WriteError::NotAFile)
    );
    let _ = file;
}

extern crate std;
use std::format;
use std::string::String;
use std::vec;
use std::vec::Vec;

/// A device of zeroed sectors, big enough for a FAT32 volume with small clusters.
struct Blank(Vec<u8>);

impl crate::sector::SectorDevice for Blank {
    type Error = u64;

    fn sector_size(&self) -> crate::SectorSize {
        crate::SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), u64> {
        let start = index as usize * 512;
        buffer.copy_from_slice(self.0.get(start..start + 512).ok_or(index)?);
        Ok(())
    }
}

impl crate::sector::WritableSectorDevice for Blank {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), u64> {
        let start = index as usize * 512;
        self.0
            .get_mut(start..start + 512)
            .ok_or(index)?
            .copy_from_slice(buffer);
        Ok(())
    }
}

/// A directory holds 65,536 entries and no more: it grows to exactly that many slots, then
/// `grow_directory` says no and a run past the last slot is `DirectoryFull`. Cluster sizes of
/// 512 and 1024 bytes put the limit at 4,096 and 2,048 clusters.
#[test]
#[cfg_attr(
    miri,
    ignore = "grows a 70 MiB volume's root to 6,144 clusters; FAT32's cluster minimum leaves no smaller volume"
)]
fn a_directory_stops_growing_at_65536_entries() {
    use crate::{FileAllocationTableType, FormatOptions, FormatPlan, SectorSize};
    for sectors_per_cluster in [1u32, 2] {
        let sectors = 140 * 1024u32;
        let plan = FormatPlan::new(
            FormatOptions::new(SectorSize::Bytes512, sectors)
                .with_file_allocation_table_type(FileAllocationTableType::Fat32)
                .with_sectors_per_cluster(sectors_per_cluster),
            &mut NoClock,
        )
        .unwrap();
        let mut device = Blank(vec![0; sectors as usize * 512]);
        plan.write_to(&mut device).unwrap();
        let mut volume = crate::Volume::open(device).unwrap();
        let wanted = 65_536 * 32 / (512 * u64::from(sectors_per_cluster));

        // The root is the FAT32 directory the loop grows; it starts with one cluster.
        let mut grown = 1u64;
        while volume.grow_directory(0).unwrap() {
            grown += 1;
            assert!(grown <= wanted, "the directory grew past 65,536 entries");
        }
        assert_eq!(grown, wanted);
        let first = match volume.boot.root_directory() {
            crate::RootDirectory::Cluster(root) => root,
            crate::RootDirectory::Region { .. } => unreachable!("a FAT32 root is a chain"),
        };
        assert_eq!(volume.chain(first).count() as u64, wanted);

        // The last slot exists and one more is refused, before any growing is tried.
        assert_eq!(volume.ensure_slots(0, MOST_SLOTS), Ok(()));
        assert_eq!(
            volume.ensure_slots(0, MOST_SLOTS + 1),
            Err(WriteError::DirectoryFull)
        );
        assert_eq!(volume.grow_directory(0), Ok(false));
        assert_eq!(volume.chain(first).count() as u64, wanted);
    }
}

/// A FAT32 root has a cluster of its own, so a caller may name it by that cluster as well as by
/// [`DirectoryCluster::ROOT`]. Either way a directory made in it records its parent as 0 in `..`,
/// the number the specification gives the root.
#[test]
#[cfg_attr(
    miri,
    ignore = "formats a 70 MiB volume; FAT32's cluster minimum leaves no smaller one"
)]
fn a_fat32_root_named_by_its_cluster_is_still_0_in_dotdot() {
    use crate::{FileAllocationTableType, FormatOptions, FormatPlan, SectorSize};
    let sectors = 140 * 1024u32;
    let plan = FormatPlan::new(
        FormatOptions::new(SectorSize::Bytes512, sectors)
            .with_file_allocation_table_type(FileAllocationTableType::Fat32),
        &mut NoClock,
    )
    .unwrap();
    let mut device = Blank(vec![0; sectors as usize * 512]);
    plan.write_to(&mut device).unwrap();
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let root = match volume.boot.root_directory() {
        crate::RootDirectory::Cluster(root) => root,
        crate::RootDirectory::Region { .. } => unreachable!("a FAT32 root is a chain"),
    };
    assert_ne!(root, 0);
    let made = volume
        .create_directory(DirectoryCluster(root), "Sub")
        .unwrap();
    let dotdot = volume.read_slot(made.first_cluster(), 1).unwrap().unwrap();
    assert_eq!(&dotdot[..11], b"..         ");
    assert_eq!(
        (
            u16::from_le_bytes([dotdot[20], dotdot[21]]),
            u16::from_le_bytes([dotdot[26], dotdot[27]])
        ),
        (0, 0)
    );
    assert!(volume.lookup("Sub").is_ok());
}

/// The small FAT12 volume, counting the sectors it writes.
struct Counted {
    image: Image,
    writes: u32,
}

impl crate::sector::SectorDevice for Counted {
    type Error = u64;

    fn sector_size(&self) -> crate::SectorSize {
        crate::SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), u64> {
        self.image.read_sector(index, buffer)
    }
}

impl crate::sector::WritableSectorDevice for Counted {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), u64> {
        self.writes += 1;
        self.image.write_sector(index, buffer)
    }
}

/// A short entry and the end marker after it, in one sector, go down in one write.
#[test]
fn an_end_marker_beside_the_new_entry_rides_in_its_write() {
    let device = Counted {
        image: Image::fat12(),
        writes: 0,
    };
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let mut stale = [0u8; SLOT_BYTES];
    stale[..11].copy_from_slice(b"STALE   TXT");
    volume.write_slot(0, 1, &stale).unwrap();
    let before = volume.device.writes;
    volume.create_file(DirectoryCluster::ROOT, "A.TXT").unwrap();
    assert_eq!(volume.device.writes - before, 1);
    assert_eq!(volume.read_slot(0, 1).unwrap().unwrap()[0], 0);
}

/// A directory whose name is taken is refused before a cluster is taken for it: nothing is
/// written, not even the dot entries.
#[test]
fn a_taken_directory_name_is_refused_before_anything_is_written() {
    let device = Counted {
        image: Image::fat12(),
        writes: 0,
    };
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    volume
        .create_directory(DirectoryCluster::ROOT, "Sub Directory")
        .unwrap();
    let before = volume.device.writes;
    assert_eq!(
        volume.create_directory(DirectoryCluster::ROOT, "SUB DIRECTORY"),
        Err(WriteError::AlreadyExists)
    );
    assert_eq!(volume.device.writes, before);
}

/// **A rename is the one operation a crash can leave sharing a chain.** It writes the new entry,
/// then (for a moved directory) `..`, then frees the old slots, so a power cut between the first
/// write and the last leaves two entries naming one chain, which `fsck.fat` reports as shared
/// clusters. The other order would lose the file on the same cut. This pins the order the README's
/// `BUGS` states: after a cut at any write, the file is listed under one name or both, never
/// neither, and two listings share their first cluster.
#[test]
fn a_power_cut_during_a_rename_leaves_one_entry_or_two_on_one_chain_and_never_none() {
    use crate::reader::tests::{POWER_GONE, PowerCut};
    let mut volume = WritableVolume::open(Image::fat12(), NoClock).unwrap();
    volume
        .create_directory(DirectoryCluster::ROOT, "SUB")
        .unwrap();
    let entry = volume.create_file(DirectoryCluster::ROOT, "A.BIN").unwrap();
    volume
        .open_file_writer(&entry)
        .unwrap()
        .write(b"data")
        .unwrap();
    let before = volume.into_device().bytes;

    let mut writes = 0;
    let mut cut_with_both = false;
    loop {
        // A rename takes a handful of writes; more than this means it never finishes.
        assert!(writes < 64, "no rename finished within 64 writes");
        let image = Image {
            bytes: before,
            refuse: None,
            refuse_write: None,
        };
        let mut volume = WritableVolume::open(PowerCut::after(image, writes), NoClock).unwrap();
        let entry = volume.lookup("A.BIN").unwrap();
        let sub = volume
            .lookup("SUB")
            .unwrap()
            .directory_cluster()
            .expect("a directory");
        let renamed = volume.rename(&entry, sub, "B.BIN");
        // Only the cut stops the rename, so the loop ends at the first budget it fits in.
        if let Err(error) = &renamed {
            assert_eq!(
                *error,
                WriteError::Chain(ChainError::Device(POWER_GONE)),
                "a cut after {writes} writes"
            );
        }
        let mut volume = crate::Volume::open(volume.into_device().image).unwrap();
        let old = volume.lookup("A.BIN").ok();
        let new = volume.lookup("SUB/B.BIN").ok();
        match (&old, &new) {
            (None, None) => panic!("a cut after {writes} writes lost the file"),
            (Some(old), Some(new)) => {
                assert_eq!(old.first_cluster(), new.first_cluster());
                assert!(renamed.is_err(), "a finished rename left the old entry");
                cut_with_both = true;
            }
            _ => {}
        }
        if renamed.is_ok() {
            assert!(old.is_none() && new.is_some());
            break;
        }
        writes += 1;
    }
    assert!(
        cut_with_both,
        "no cut left both entries, so the claim needs no exception"
    );
}
