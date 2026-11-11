# file_allocation_table

[![check](https://github.com/nifeos/file_allocation_table/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nifeos/file_allocation_table/actions/workflows/ci.yml?query=branch%3Amain)
[![proofs](https://github.com/nifeos/file_allocation_table/actions/workflows/proofs.yml/badge.svg?branch=main)](https://github.com/nifeos/file_allocation_table/actions/workflows/proofs.yml?query=branch%3Amain)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/nifeos/file_allocation_table#license)
[![minimum Rust version](https://img.shields.io/badge/rust-1.88%2B-blue.svg)](https://github.com/nifeos/file_allocation_table/blob/main/Cargo.toml)

File Allocation Table (FAT) volumes as pure computation, in all three widths: FAT12, FAT16 and
FAT32. The crate reads, writes and formats volumes through a sector interface you implement. It does
no I/O, needs no allocator (`no_std`), and names no device. It reads and writes the long file
names that the Virtual File Allocation Table (VFAT) extension adds. Kani, a model checker that
proves properties for all inputs up to a stated size, checks every function that parses bytes read
from a disk.

## What it does

- Read. `Volume::open` checks the boot sector, types the volume by its cluster count as the
  specification does, and reads directories, long names, paths and file bytes. A `MasterBootRecord`
  opens the volume inside a partitioned disk for reading; its `PartitionDevice` has no write path.
- Write. Open a `WritableVolume` with a `Clock`; a read-only `Volume` has no write methods and
  takes no clock. Create, write, extend, truncate, rename and remove files; make and remove
  directories; set attributes (read-only, hidden, system, archive); read and change the volume
  label. A name that is not 8.3 gets long-name slots and an alias numbered against its directory.
  The first change clears the clean-shutdown bit, and `WritableVolume::mark_clean` sets it again
  and updates the hints in the FAT32 file system information sector.
- Format. `FormatPlan` lays out an empty volume of any size and sector size (512 to 4096 bytes),
  with the cluster size, serial number and label of your choice. Left to choose, it picks the type
  and the cluster size by the volume's size from the specification's tables, as Windows does.
  That is FAT12 to about 4 MiB, then FAT16 below 512 MiB with clusters of 1 to 8 KiB. FAT32 takes
  over from there with 4 KiB clusters, growing to 32 KiB above 32 GiB. `mkfs.fat` picks the same
  FAT32 clusters. `VolumeLayout` lays out the one
  shape a bootloader needs: a FAT32 Extensible Firmware Interface (EFI) system partition holding one
  boot file, with 512-byte sectors and 4096-byte clusters. The `efi_system_partition` module holds
  that shape's constants.

## EXAMPLES

```rust
use file_allocation_table::{
    Attributes, DirectoryCluster, FormatOptions, FormatPlan, NoClock, SectorDevice, SectorSize,
    WritableSectorDevice, WritableVolume,
};

/// A volume held in memory. A real caller wraps a block device the same way.
struct Memory(Vec<u8>);

impl SectorDevice for Memory {
    type Error = ();

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), ()> {
        let start = usize::try_from(index).map_err(|_| ())? * 512;
        buffer.copy_from_slice(self.0.get(start..start + 512).ok_or(())?);
        Ok(())
    }
}

impl WritableSectorDevice for Memory {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), ()> {
        let start = usize::try_from(index).map_err(|_| ())? * 512;
        self.0
            .get_mut(start..start + 512)
            .ok_or(())?
            .copy_from_slice(buffer);
        Ok(())
    }
}

fn main() {
    // Format 8 MiB. The size makes it FAT16 with 1 KiB clusters; about 4 MiB or less makes FAT12,
    // and 512 MiB or more makes FAT32.
    let sectors = 16 * 1024;
    let options = FormatOptions::new(SectorSize::Bytes512, sectors)
        .with_label("EXAMPLE")
        .unwrap();
    let format = FormatPlan::new(options, &mut NoClock).expect("a legal size");
    let mut device = Memory(vec![0; sectors as usize * 512]);
    format.write_to(&mut device).unwrap();

    // Open it, make a directory and a file with a long name, and write some bytes.
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let docs = volume
        .create_directory(DirectoryCluster::ROOT, "Documents")
        .unwrap();
    let file = volume
        .create_file(docs.directory_cluster().unwrap(), "A long file name.txt")
        .unwrap();
    volume
        .open_file_writer(&file)
        .unwrap()
        .write(b"hello, volume")
        .unwrap();
    let stored = volume.lookup("Documents/A long file name.txt").unwrap();
    let hidden = Attributes::ARCHIVE.union(Attributes::HIDDEN);
    volume.set_attributes(&stored, hidden).unwrap();
    volume.mark_clean().unwrap();

    // Read it back by path, as another program would.
    let entry = volume.lookup("documents/A LONG FILE NAME.TXT").unwrap();
    let mut bytes = [0u8; 32];
    let read = volume
        .open_file_reader(&entry)
        .unwrap()
        .read(&mut bytes)
        .unwrap();
    assert_eq!(&bytes[..read], b"hello, volume");
    assert_eq!(
        volume.volume_label().unwrap().unwrap().trim_ascii_end(),
        b"EXAMPLE"
    );
}
```

A documentation test compiles and runs this example. Every write goes through a `WritableVolume`,
which needs a `Clock`. `NoClock` is the explicit "no time", and the entries it stamps carry the zero
timestamp. `FormatPlan` formats any FAT volume onto a device, and a `WritableVolume` then writes its
files. `VolumeLayout` is for one shape, an EFI system partition, written without a device in one
pass. `VolumeLayout::new` takes a `VolumeLayoutOptions` and a clock. The options hold the total
sector count, and optionally the hidden sectors, the volume identifier, a label you choose, such as
`"EFI SYSTEM"`, and the one boot file's name and size. The layout then tells you what every metadata
sector holds. Write sectors `0..metadata_sectors()` from `fill_sector`, then the file's bytes from
`file_first_sector()` on, `file_sectors()` long. Without a label the volume has none, and without a
file it is an empty FAT32 volume.

A `PartitionDevice` only reads. To write inside a partition, wrap the whole-disk device in a small
device of your own. It adds the partition's first sector to every index, and the volume opens
through it. The example below does this.

```rust
use file_allocation_table::{
    DirectoryCluster, NoClock, Partition, SectorDevice, SectorSize, WritableSectorDevice,
    WritableVolume,
};

/// Shows a disk from sector `first` on, as a device whose sector 0 is that sector.
struct Offset<D> {
    device: D,
    first: u64,
}

impl<D: SectorDevice> SectorDevice for Offset<D> {
    type Error = D::Error;

    fn sector_size(&self) -> SectorSize {
        self.device.sector_size()
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), D::Error> {
        self.device.read_sector(self.first + index, buffer)
    }
}

impl<D: WritableSectorDevice> WritableSectorDevice for Offset<D> {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), D::Error> {
        self.device.write_sector(self.first + index, buffer)
    }
}

/// Make a file in the volume inside `partition`, which `MasterBootRecord::partitions` yielded.
fn create_in_partition<D: WritableSectorDevice>(disk: D, partition: Partition) {
    let first = u64::from(partition.first_sector());
    let Ok(mut volume) = WritableVolume::open(Offset { device: disk, first }, NoClock) else {
        return;
    };
    let _ = volume.create_file(DirectoryCluster::ROOT, "NOTE.TXT");
}
```

This `Offset` does not stop a write past the partition's last sector, which would land in the next
partition. A caller that cannot trust the volume's boot sector checks each index against
`Partition::sector_count` before it writes.

Every error enum is `#[non_exhaustive]`, so a match on one needs a wildcard arm, and a later
release can add a variant without breaking your code. Every error type implements `Display` and
`core::error::Error`, with no allocator. A message names its own level only, and `source()` returns
the error it wraps. A type generic over your device's error displays for any error, and is an
`Error`, so `?` takes it into a boxed error, when your device's error is one.

## Footprint

The crate allocates nothing; every byte below lives where the caller puts the value or on the stack
of the call. A `Volume` or a `WritableVolume` is 4,192 bytes plus the device, most of it one sector
buffer of 4,096 bytes, the largest sector size. A `Directory` or a `FileReader` borrows the volume
and holds its own buffer, 4,152 bytes each. A `FileWriter` is 88 bytes, a `NamedEntry` 576 (it
holds the long name), a `FormatPlan` 64 and a `VolumeLayout` 624. These are the sizes on a 64-bit
host. The write paths put sector-sized arrays on the stack for each call. Measured on x86-64 in a
release build, a thread that formats a volume and then creates a directory and a file, writes
10,000 bytes, renames, looks up and reads needed about 40 KiB of stack, the volume itself included.

## How it compares

- rust-fatfs (the `fatfs` crate) is the most used FAT crate. It has `std::io` traits, a time
  provider, code-page conversion for short names and years of users, and this crate's tests
  compare against it. This crate adds FAT12 writing and formatting under `no_std` with no allocator,
  no `unsafe`, and a refusal, never a panic, on a hostile image.
- embedded-sdmmc brings a Secure Digital (SD) card driver, a block cache and `embedded-io` traits, over 512-byte
  blocks. This crate has no driver. It adds FAT12, long-name creation, a formatter and sector
  sizes up to 4,096 bytes.
- hadris-fat is the nearest design: its `hadris-fat-raw` holds codecs free of I/O with Kani proofs,
  and the family adds exFAT and a checker. This crate reads, writes and formats all three widths
  in one crate, with a falsification patch for every proof, and lays out an EFI system partition.
- What this crate lacks: exFAT, `std::io` and `embedded-io` traits, code-page conversion for short
  names, and the users the crates above have.

## What is proven

Kani harnesses prove, over every input they can unroll, the properties below.

`cargo build` builds the crate and `cargo test` runs its unit tests, integration tests and
documentation tests in well under a minute. The proofs need Kani, which `rustup` does not install.
Run `cargo install --locked kani-verifier --version 0.68.0`, then `cargo kani setup`. The setup
step downloads a model checker, so it needs network access. CI uses version 0.68.0, and another
version may verify differently. After that, `cargo kani` from the crate's directory runs every
harness; the whole run takes 10 to 20 minutes, and `cargo kani --harness <name>` runs one.

- Hostile input. No parse step panics or reads outside the volume: the boot sector and its
  geometry fields for every 512-byte sector, FAT entries, directory slots, long-name runs, the
  partition table and the file system information sector. A cluster chain walk cannot loop. It ends
  within the volume's cluster count.
- Writes. A stored table entry reads back and disturbs nothing else. Allocation takes one free
  cluster and changes nothing else. A release frees only the chain it received and never a bad
  cluster. A cut keeps exactly the head of the chain. A partial sector write keeps the head, zeroes
  the slack and lands the bytes. A write extends the chain to the size it needs.
- Names. The long-name checksum equals the specification's loop for every short name. Every
  long-name run is well formed. Every alias is a legal short name that carries its number, and
  reads back to that number, which is how one read of a directory finds every number taken. The
  first free number of an alias window is the least one not marked. The finder takes the first
  run of free slots.
- Format. A geometry the format computes parses back as the same type, clusters, table size
  and data start.
- Attributes and labels. Setting attributes changes the four user bits and nothing else, and
  never makes a long-name piece; an accepted label is legal bytes then spaces.

Each harness has a falsification record at `falsifications/<module.path>.<harness>.patch`. Applying
it with `patch -p1` from the crate root makes that harness fail at its own assertion, which shows
the proof checks something. Kani bounds what it unrolls: the model volumes are a few clusters, and
the format proof covers clusters of 1 to 8 sectors. The tests against rust-fatfs cover whole images.

The tests compare against rust-fatfs (the `lamfat` dev-dependency), an independent FAT library.
Each library writes trees on FAT12, FAT16 and FAT32 volumes, and the other must read them, name for
name and byte for byte. If the host has `fsck.fat` or `fsck_msdos`, every image must pass
`fsck -n`. Where `mkfs.fat` is installed, its FAT12 and FAT16 volumes take this crate's writer. The
crate reads two FAT32 images that a macOS host formatted and wrote, to their recorded hashes. An
ignored test has the host's own driver read an image (`cargo test -- --ignored`, on macOS).

Two cargo-fuzz targets in the repository's `fuzz/` directory, which the crates.io package leaves
out, reach what the proofs bound. `file_allocation_table_boot_sector` reads an arbitrary volume at
each sector size and walks its directories, chains and files. `file_allocation_table_writer` runs
arbitrary writes, renames and removals on a hostile image or on a volume the crate formatted. Run
one with `cargo +nightly fuzz run <target>`; the weekly fuzz job runs both for a bounded time.

## BUGS

- Not crash-atomic, and there is no journal, since FAT has none. The crate orders its writes so a
  crash leaks a cluster and never shares one, with one exception. A rename writes the new entry
  before it frees the old one, so a crash between the two leaves two entries naming one chain.
  `fsck.fat` reports that as shared clusters. The other order would lose the file on the same
  crash. A directory grows by a cluster that is zeroed before it is linked, so a crash never shows
  a directory the bytes a device held there. A write has no barrier: the device decides when a
  write reaches the medium.
- Write amplification. Every new cluster costs table writes, so small clusters multiply the
  writes: measured in memory, one-sector clusters cost 5.0 sector writes per data sector and
  eight-sector clusters cost 1.5. The default cluster grows with the volume, so the higher figure
  falls on a FAT12 volume of a few MiB, or on one formatted with one-sector clusters on purpose.
- Time. Every writer needs a clock. FAT stores a date and a time of day with no time zone, and the
  crate expects the clock to supply Coordinated Universal Time (UTC). The crate stores exactly the
  fields the clock reports and converts nothing, so it cannot check that a clock gives UTC. A reader
  that treats FAT times as local time, such as Windows, shows a UTC value as local time. A read
  takes no clock and never writes. `CivilDateTime::new` refuses a date no calendar holds, and a
  year outside 1980 to 2107 fails with `TimeOutOfRange` before the crate writes a byte. A stored
  date no calendar holds reads as `None`. A clock that cannot report the time
  returns `None` and gets the zero timestamp. A write sets the write and access times, a new
  entry gets its creation time, a rename keeps the times, and a read stamps nothing. An entry's
  `created`, `modified` and `accessed` read them back: the creation time to the second, the write
  time to two seconds, and the access date at midnight, since FAT keeps no access time of day. The
  zero timestamp reads as `None`.
- An entry is a copy of its directory slot at the time it was read, and a write does not update it,
  so look the entry up again after a write. Opening a reader or a writer, a rename, a removal or
  an attribute change through a stale entry refuses with `EntryChanged`.
- The crate refuses a name with a character outside the Basic Multilingual Plane. It stores a
  lower-case 8.3 name as a long name, since it writes no lower-case flag. It reads the flags that
  Windows and Linux write: `cased_short_name` shows such a name in lower case, and `short_name`
  keeps the stored bytes. A rename drops the flags with the old name.
- Names compare by simple Unicode upper-casing, not the volume's code page or upcase table.
- A create, a rename or a new directory reads the whole directory it writes into once: one pass
  checks the name, numbers the alias and finds the free slots. Filling one directory with n
  entries therefore costs on the order of n squared sector reads, as it does in rust-fatfs. The
  figures below count the sectors read and written on a 128 MiB FAT32 volume held in memory,
  formatted with one sector to the cluster. A real device adds its own time to every sector, and the
  crate keeps no cache of directory sectors. Each pair is this writer, then the writer that read
  the directory once per candidate alias and once per slot.
  - 1,000 short (8.3) names in one directory: 34,000 reads, against 569,000.
  - 1,000 long names with distinct first characters: 133,000 reads, against 3.3 million.
  - 1,000 long names that share their first six characters, so that every alias collides:
    101,000 reads and 0.3 seconds, against 33.8 million reads and 24 seconds. rust-fatfs takes
    1.7 seconds for the same names on the same host. A test holds 500 such names to 34,000
    reads; they take 25,000, against 4.4 million.
  - An alias numbered past 1,024 costs one more read of the directory for each further 1,024
    numbers.
- Sequential writes. A new cluster's end mark and the link to it share one write to each of the
  two table copies when they sit in one table sector. A 4 MiB file written in 64 KiB pieces takes
  3.0 sector writes per data sector at one sector to the cluster, and 1.26 at eight. With the end
  mark and the link as separate writes it took 5.0 and 1.51. A test holds the first figure to
  3.1. Every `write` call also reads and rewrites the file's directory entry, so small calls cost
  more: 4 MiB in 100-byte calls takes 11.5 writes and 15.6 reads per data sector, against 11.7
  and 15.9. Pass whole sectors where you can.
- The reader does not follow extended partitions and reads no Globally Unique Identifier (GUID)
  partition table.
- exFAT is out of scope; it is a sibling crate's job.

## Minimum Rust version

Rust 1.88.

## License

Licensed under either of the MIT license ([LICENSE-MIT](LICENSE-MIT)) or the Apache License,
Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)), at your option.
