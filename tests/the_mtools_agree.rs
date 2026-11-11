//! **mtools reads what this crate writes, and this crate reads what mtools writes.**
//!
//! mtools is a FAT implementation of its own, separate from rust-fatfs and `fsck.fat`, and it runs
//! without privilege on an image file. On a FAT12, a FAT16 and a FAT32 volume this crate formats
//! and fills, `mdir` must list every name, `mtype` must print every file's bytes, `mattrib` must
//! show the hidden bit and `mlabel` the label. Then `mmd`, `mcopy` and `mattrib` write a
//! directory, a file with a long name, a lower-case 8.3 name and a read-only bit, and this crate
//! must find and read them all, and write beside them a file `mdir` lists. A host without mtools
//! reports that and skips the comparison.

// Not under Miri: this suite runs the host's mtools on images in temporary files, which Miri's
// isolation refuses. The crate has no `unsafe`, and its library unit tests run under Miri.
#![cfg(not(miri))]

use std::path::{Path, PathBuf};
use std::process::Command;

use file_allocation_table::{
    Attributes, DirectoryCluster, FileAllocationTableType, FormatOptions, FormatPlan, NoClock,
    SectorDevice, SectorSize, WritableSectorDevice, WritableVolume,
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

/// The three volumes: a name for messages, the type, and the size in 512-byte sectors.
const VOLUMES: [(&str, FileAllocationTableType, u32); 3] = [
    ("fat12", FileAllocationTableType::Fat12, 4 * 1024),
    ("fat16", FileAllocationTableType::Fat16, 32 * 1024),
    ("fat32", FileAllocationTableType::Fat32, 128 * 1024),
];

/// The bytes of the file with a long name.
const LONG_BYTES: &[u8] = b"written by the crate, read by mtools\n";

/// Whether `mdir` runs on this host; says which way the suite goes.
fn mtools_present() -> bool {
    let present = Command::new("mdir").arg("-V").output().is_ok();
    if present {
        eprintln!("mtools is present; the comparison runs");
    } else {
        eprintln!("no mtools on this host; the comparison is skipped");
    }
    present
}

/// An image file for `label` in the temporary directory, holding `bytes`.
fn image_file(label: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "file-allocation-table-mtools-{label}-{}.img",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write the image for mtools");
    path
}

/// Run the mtools command `tool` on `image` with `arguments`; it must succeed. Its standard output.
fn mtools(tool: &str, image: &Path, arguments: &[&str]) -> String {
    let output = Command::new(tool)
        .arg("-i")
        .arg(image)
        .args(arguments)
        // No configuration file of the host's or the user's changes what the tools do.
        .env("MTOOLSRC", "/dev/null")
        .output()
        .expect("run mtools");
    assert!(
        output.status.success(),
        "{tool} {arguments:?} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("mtools prints UTF-8")
}

/// A formatted volume of `sectors` of `kind`, labeled, with a directory, a file with a long name
/// inside it and a hidden short-named file in the root.
fn filled(kind: FileAllocationTableType, sectors: u32) -> Memory {
    let options = FormatOptions::new(SectorSize::Bytes512, sectors)
        .with_file_allocation_table_type(kind)
        .with_label("MTOOLS")
        .expect("a legal label");
    let plan = FormatPlan::new(options, &mut NoClock).expect("a legal size");
    let mut device = Memory(vec![0; sectors as usize * 512]);
    plan.write_to(&mut device).expect("the format writes");
    let mut volume = WritableVolume::open(device, NoClock).expect("the volume opens");
    let documents = volume
        .create_directory(DirectoryCluster::ROOT, "Documents")
        .expect("the directory is created");
    let long = volume
        .create_file(
            documents.directory_cluster().expect("a directory"),
            "A long file name.txt",
        )
        .expect("the file is created");
    volume
        .open_file_writer(&long)
        .expect("the file opens")
        .write(LONG_BYTES)
        .expect("the bytes are written");
    let hidden = volume
        .create_file(DirectoryCluster::ROOT, "HIDDEN.TXT")
        .expect("the file is created");
    volume
        .set_attributes(&hidden, Attributes::ARCHIVE.union(Attributes::HIDDEN))
        .expect("the bits are set");
    volume.mark_clean().expect("the volume is marked clean");
    volume.into_device()
}

#[test]
fn mtools_lists_and_reads_every_name_the_crate_writes() {
    if !mtools_present() {
        return;
    }
    for (label, kind, sectors) in VOLUMES {
        let path = image_file(&format!("{label}-written"), &filled(kind, sectors).0);
        let listing = mtools("mdir", &path, &["-/", "-a", "::"]);
        for name in ["Documents", "A long file name.txt", "HIDDEN"] {
            assert!(
                listing.contains(name),
                "{label}: mdir lists {name}:\n{listing}"
            );
        }
        let bytes = mtools("mtype", &path, &["::/Documents/A long file name.txt"]);
        assert_eq!(
            bytes.as_bytes(),
            LONG_BYTES,
            "{label}: mtype reads the file"
        );
        let bits = mtools("mattrib", &path, &["::/HIDDEN.TXT"]);
        assert!(
            bits.split_whitespace().any(|field| field == "H"),
            "{label}: mattrib shows the hidden bit: {bits}"
        );
        let name = mtools("mlabel", &path, &["-s", "::"]);
        assert!(
            name.contains("MTOOLS"),
            "{label}: mlabel reads the label: {name}"
        );
        let _ = std::fs::remove_file(&path);
        eprintln!("{label}: mtools read every name the crate wrote");
    }
}

#[test]
fn the_crate_reads_what_mtools_writes_and_writes_beside_it() {
    if !mtools_present() {
        return;
    }
    let source = std::env::temp_dir().join(format!(
        "file-allocation-table-mtools-source-{}.txt",
        std::process::id()
    ));
    let bytes = b"written by mtools, read by the crate\n";
    std::fs::write(&source, bytes).expect("write the source file");
    let source_name = source.to_str().expect("a UTF-8 path");
    for (label, kind, sectors) in VOLUMES {
        let options =
            FormatOptions::new(SectorSize::Bytes512, sectors).with_file_allocation_table_type(kind);
        let plan = FormatPlan::new(options, &mut NoClock).expect("a legal size");
        let mut device = Memory(vec![0; sectors as usize * 512]);
        plan.write_to(&mut device).expect("the format writes");
        let path = image_file(&format!("{label}-read"), &device.0);
        mtools("mmd", &path, &["::/Made by mtools"]);
        mtools(
            "mcopy",
            &path,
            &[source_name, "::/Made by mtools/A copied file.txt"],
        );
        mtools("mcopy", &path, &[source_name, "::/lower.txt"]);
        mtools("mattrib", &path, &["+r", "::/lower.txt"]);

        let image = std::fs::read(&path).expect("read the image back");
        let mut volume = WritableVolume::open(Memory(image), NoClock).expect("the volume opens");
        for name in ["Made by mtools/A copied file.txt", "lower.txt"] {
            let entry = volume.lookup(name).expect("the crate finds the file");
            let mut read = [0u8; 64];
            let got = volume
                .open_file_reader(&entry)
                .expect("the file opens")
                .read(&mut read)
                .expect("the file reads");
            assert_eq!(
                &read[..got],
                bytes,
                "{label}: {name} reads as mtools wrote it"
            );
        }
        let lower = volume
            .lookup("lower.txt")
            .expect("the crate finds the file");
        assert_ne!(
            lower.attributes() & Attributes::READ_ONLY.bits(),
            0,
            "{label}: the read-only bit mattrib set reads here"
        );
        let directory = volume
            .lookup("Made by mtools")
            .expect("the crate finds the directory")
            .directory_cluster()
            .expect("a directory");
        volume
            .create_file(directory, "Written beside it.txt")
            .expect("the crate writes beside the files mtools wrote");
        volume.mark_clean().expect("the volume is marked clean");
        std::fs::write(&path, volume.into_device().0).expect("write the image back");
        let listing = mtools("mdir", &path, &["-/", "-a", "::"]);
        assert!(
            listing.contains("Written beside it.txt"),
            "{label}: mdir lists the file the crate wrote:\n{listing}"
        );
        let _ = std::fs::remove_file(&path);
        eprintln!("{label}: the crate read what mtools wrote, and mtools read what it added");
    }
    let _ = std::fs::remove_file(&source);
}
