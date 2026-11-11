//! **A FAT32 volume under 65525 clusters opens when the caller asks, and only then.**
//!
//! `mkfs.fat -F 32` makes such a volume with a warning, and Linux mounts it as FAT32. The strict
//! open refuses it. With `OpenOptions::with_small_fat32`, this crate opens it, writes a directory
//! and a file of several clusters into it, and `fsck.fat -n` accepts the result, which this crate
//! then reads back. A host without `mkfs.fat` reports that and skips the test.

// Not under Miri: this suite runs the host's `mkfs.fat` and `fsck` on images in temporary files,
// which Miri's isolation refuses. The crate has no `unsafe`, and its library unit tests run under
// Miri.
#![cfg(not(miri))]

use std::path::PathBuf;
use std::process::Command;

use file_allocation_table::{
    DirectoryCluster, FileAllocationTableType, NoClock, OpenOptions, ParseError, ReadError,
    SectorDevice, SectorSize, Volume, WritableSectorDevice, WritableVolume,
};

/// A volume held in memory, read and written one 512-byte sector at a time.
struct Memory(Vec<u8>);

impl SectorDevice for Memory {
    type Error = u64;

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), u64> {
        let start = usize::try_from(index).map_err(|_| index)? * 512;
        buffer.copy_from_slice(self.0.get(start..start + 512).ok_or(index)?);
        Ok(())
    }
}

impl WritableSectorDevice for Memory {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), u64> {
        let start = usize::try_from(index).map_err(|_| index)? * 512;
        self.0
            .get_mut(start..start + 512)
            .ok_or(index)?
            .copy_from_slice(buffer);
        Ok(())
    }
}

/// The first of `tools` that runs on this host.
fn first_present(tools: &[&'static str]) -> Option<&'static str> {
    tools
        .iter()
        .copied()
        .find(|tool| Command::new(tool).arg("--help").output().is_ok())
}

/// The temporary image file for `label`.
fn image_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "file-allocation-table-small-fat32-{label}-{}.img",
        std::process::id()
    ))
}

/// The bytes of the file written into the volume: 3000 clusters of 512 bytes, so its chain crosses
/// several table sectors.
fn contents() -> Vec<u8> {
    (0..3000 * 512).map(|at: u32| (at % 251) as u8).collect()
}

#[test]
fn a_small_fat32_volume_opens_takes_files_and_passes_fsck() {
    let Some(mkfs) = first_present(&["mkfs.fat", "/usr/sbin/mkfs.fat", "/sbin/mkfs.fat"]) else {
        eprintln!("no mkfs.fat on this host; the small FAT32 volume is skipped");
        return;
    };
    // 10 MiB at one sector per cluster: about 20,000 clusters, a third of the FAT32 floor.
    let path = image_path("made");
    std::fs::File::create(&path)
        .and_then(|file| file.set_len(10 << 20))
        .expect("create the image");
    let made = Command::new(mkfs)
        .args(["-F", "32", "-s", "1"])
        .arg(&path)
        .output()
        .expect("run mkfs.fat");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let bytes = std::fs::read(&path).expect("read the image");
    let _ = std::fs::remove_file(&path);

    assert!(matches!(
        Volume::open(Memory(bytes.clone())),
        Err(ReadError::Refused(
            ParseError::LayoutContradictsClusterCount
        ))
    ));
    let options = OpenOptions::new().with_small_fat32(true);
    let mut volume = WritableVolume::open_with(Memory(bytes), NoClock, options).unwrap();
    let boot = *volume.boot_sector();
    assert_eq!(
        boot.file_allocation_table_type(),
        FileAllocationTableType::Fat32
    );
    assert!(boot.clusters() < 65_525, "{} clusters", boot.clusters());

    let directory = volume
        .create_directory(DirectoryCluster::ROOT, "Written Here")
        .unwrap();
    let file = volume
        .create_file(
            directory.directory_cluster().unwrap(),
            "several clusters.bin",
        )
        .unwrap();
    let written = contents();
    {
        let mut writer = volume.open_file_writer(&file).unwrap();
        let mut done = 0;
        while done < written.len() {
            done += writer.write(&written[done..]).unwrap();
        }
    }
    volume.mark_clean().unwrap();
    let image = volume.into_device().0;

    match first_present(&["fsck.fat", "/usr/sbin/fsck.fat", "/sbin/fsck.fat"]) {
        Some(fsck) => {
            let path = image_path("written");
            std::fs::write(&path, &image).expect("write the image for fsck");
            let checked = Command::new(fsck)
                .arg("-n")
                .arg(&path)
                .output()
                .expect("run fsck");
            let _ = std::fs::remove_file(&path);
            assert!(
                checked.status.success(),
                "{fsck} -n refused the image:\n{}{}",
                String::from_utf8_lossy(&checked.stdout),
                String::from_utf8_lossy(&checked.stderr)
            );
        }
        None => eprintln!("no fsck.fat on this host; the fsck check is skipped"),
    }

    let mut volume = Volume::open_with(Memory(image), options).unwrap();
    let entry = volume
        .lookup("Written Here/several clusters.bin")
        .expect("the file is found");
    assert_eq!(entry.size_bytes(), written.len() as u32);
    let mut read = vec![0u8; written.len()];
    let mut reader = volume.open_file_reader(&entry).unwrap();
    let mut done = 0;
    while done < read.len() {
        let got = reader.read(&mut read[done..]).unwrap();
        assert_ne!(got, 0, "the file ends early at byte {done}");
        done += got;
    }
    assert!(read == written, "the file reads back as written");
}
