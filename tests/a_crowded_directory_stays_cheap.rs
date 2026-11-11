//! **What a crowded directory and a long write cost, in sector reads and writes, has a ceiling.**
//!
//! A 128 MiB FAT32 volume held in memory, at the default cluster of one sector, counts every sector
//! the crate reads and writes. Five hundred long names that share their first six characters, so
//! every alias collides with the ones before it, go into one directory; then a 4 MiB file is
//! written in 64 KiB pieces. Each count must stay under its ceiling, so a change that brings back
//! a read of the directory per candidate alias, or a table write per entry, fails here. rust-fatfs
//! must list every name and `fsck -n` must accept the volume.
//!
//! The ceilings sit about a third above what the crate measures, and two orders of magnitude
//! below what it cost before one read of the directory served a whole create: 4.4 million reads
//! for the names, and five writes for each data sector.

// Not under Miri: this suite writes a 128 MiB volume and runs the host's `fsck` through a
// temporary file, which Miri's isolation refuses. The crate has no `unsafe`.
#![cfg(not(miri))]

use std::cell::Cell;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;

use file_allocation_table::{
    DirectoryCluster, FileAllocationTableType, FormatOptions, FormatPlan, NoClock, SectorDevice,
    SectorSize, WritableSectorDevice, WritableVolume,
};

/// The volume's size in 512-byte sectors: 128 MiB.
const SECTORS: u32 = 256 * 1024;

/// How many names go into the directory.
const NAMES: usize = 500;

/// The most sector reads the five hundred creates may take. Measured: 24,994.
const MOST_NAME_READS: u64 = 34_000;

/// The most sector writes the five hundred creates may take. Measured: 841.
const MOST_NAME_WRITES: u64 = 1_100;

/// The bytes of the long file.
const FILE_BYTES: usize = 4 << 20;

/// The most sector writes per data sector the long file may take: one for the data and one for
/// each of the two table copies, plus the directory entry once per piece. Measured: 3.02.
const MOST_WRITES_PER_DATA_SECTOR: f64 = 3.1;

/// The sector reads and writes a device has served, shared with the test that reads them while
/// the volume holds the device.
#[derive(Clone, Default)]
struct Counts(Rc<Cell<(u64, u64)>>);

impl Counts {
    fn reads(&self) -> u64 {
        self.0.get().0
    }

    fn writes(&self) -> u64 {
        self.0.get().1
    }
}

/// A volume held in memory that counts what it serves.
struct Counting {
    bytes: Vec<u8>,
    counts: Counts,
}

impl SectorDevice for Counting {
    type Error = u64;

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), u64> {
        let start = usize::try_from(index).map_err(|_| index)? * 512;
        buffer.copy_from_slice(self.bytes.get(start..start + 512).ok_or(index)?);
        let (reads, writes) = self.counts.0.get();
        self.counts.0.set((reads + 1, writes));
        Ok(())
    }
}

impl WritableSectorDevice for Counting {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), u64> {
        let start = usize::try_from(index).map_err(|_| index)? * 512;
        self.bytes
            .get_mut(start..start + 512)
            .ok_or(index)?
            .copy_from_slice(buffer);
        let (reads, writes) = self.counts.0.get();
        self.counts.0.set((reads, writes + 1));
        Ok(())
    }
}

/// The `fsck` this host has, if any.
fn fsck() -> Option<&'static str> {
    [
        "fsck.fat",
        "/usr/sbin/fsck.fat",
        "/sbin/fsck.fat",
        "fsck_msdos",
    ]
    .into_iter()
    .find(|tool| Command::new(tool).arg("--help").output().is_ok())
}

/// Run the host's `fsck -n` on `image`, which must exit 0; report and skip when there is none.
fn check_with_fsck(image: &[u8]) {
    let Some(tool) = fsck() else {
        eprintln!("no fsck.fat or fsck_msdos on this host; the fsck check is skipped");
        return;
    };
    let path = std::env::temp_dir().join(format!(
        "file-allocation-table-crowded-{}.img",
        std::process::id()
    ));
    std::fs::write(&path, image).expect("write the image for fsck");
    let output = Command::new(tool)
        .arg("-n")
        .arg(&path)
        .output()
        .expect("run fsck");
    let _ = std::fs::remove_file(Path::new(&path));
    assert!(
        output.status.success(),
        "{tool} -n refused the image:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{tool} -n accepted the image");
}

/// A formatted, empty volume, and the counts of what it reads and writes from here on.
fn volume() -> (WritableVolume<Counting, NoClock>, Counts) {
    let plan = FormatPlan::new(
        // The FAT32 volume at one sector to the cluster that the README's figures count.
        FormatOptions::new(SectorSize::Bytes512, SECTORS)
            .with_file_allocation_table_type(FileAllocationTableType::Fat32)
            .with_sectors_per_cluster(1),
        &mut NoClock,
    )
    .expect("a legal size");
    let mut device = Counting {
        bytes: vec![0; SECTORS as usize * 512],
        counts: Counts::default(),
    };
    plan.write_to(&mut device).expect("the format writes");
    let counts = Counts::default();
    device.counts = counts.clone();
    let volume = WritableVolume::open(device, NoClock).expect("the volume opens");
    (volume, counts)
}

#[test]
fn five_hundred_colliding_long_names_stay_under_their_read_ceiling() {
    let (mut volume, counts) = volume();
    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "many")
        .unwrap()
        .directory_cluster()
        .unwrap();
    let (reads, writes) = (counts.reads(), counts.writes());
    for i in 0..NAMES {
        volume
            .create_file(directory, &format!("file number {i}.txt"))
            .unwrap();
    }
    let reads = counts.reads() - reads;
    let writes = counts.writes() - writes;
    eprintln!("{NAMES} colliding long names: {reads} sector reads, {writes} sector writes");
    assert!(
        reads <= MOST_NAME_READS,
        "{NAMES} creates read {reads} sectors, over the ceiling of {MOST_NAME_READS}"
    );
    assert!(
        writes <= MOST_NAME_WRITES,
        "{NAMES} creates wrote {writes} sectors, over the ceiling of {MOST_NAME_WRITES}"
    );
    // The aliases count up from 1, so the last of the five hundred carries ~500.
    let last = volume
        .lookup(&format!("many/file number {}.txt", NAMES - 1))
        .unwrap();
    assert_eq!(last.short_name(), *b"FILE~500TXT");

    volume.mark_clean().unwrap();
    let image = volume.into_device().bytes;
    let mut names = lamfat::FileSystem::new(Cursor::new(image.clone()), lamfat::FsOptions::new())
        .unwrap()
        .root_dir()
        .open_dir("many")
        .unwrap()
        .iter()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name != "." && name != "..")
        .collect::<Vec<_>>();
    names.sort();
    let mut expected = (0..NAMES)
        .map(|i| format!("file number {i}.txt"))
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(names, expected);
    check_with_fsck(&image);
}

#[test]
fn a_long_write_stays_under_its_write_ceiling() {
    let (mut volume, counts) = volume();
    let file = volume
        .create_file(DirectoryCluster::ROOT, "BIG.BIN")
        .unwrap();
    let writes = counts.writes();
    let data = vec![7u8; FILE_BYTES];
    let mut writer = volume.open_file_writer(&file).unwrap();
    for piece in data.chunks(64 * 1024) {
        assert_eq!(writer.write(piece).unwrap(), piece.len());
    }
    let writes = counts.writes() - writes;
    let per_sector = writes as f64 / (FILE_BYTES / 512) as f64;
    eprintln!("a 4 MiB write: {writes} sector writes, {per_sector:.2} per data sector");
    assert!(
        per_sector <= MOST_WRITES_PER_DATA_SECTOR,
        "{per_sector:.2} writes per data sector, over the ceiling of {MOST_WRITES_PER_DATA_SECTOR}"
    );
    volume.mark_clean().unwrap();
    check_with_fsck(&volume.into_device().bytes);
}
