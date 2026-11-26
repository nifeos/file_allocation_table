# Changelog

Dates are UTC. One line per release, what changed and why, so a reader of the diff has the
sentence beside it.

## Unreleased

- Breaking: `ShortEntry::attributes` and `NamedEntry::attributes`, the stored byte, are gone from
  the interface. `user_attributes` on both gives the `Attributes` set, which now has `|`, `&` and
  `!`.
- Breaking: a cluster number is a `Cluster`, not a bare `u32`, so it cannot be passed where a
  sector index or a count belongs. A `DirectoryCluster` converts into one.
- Breaking: both image builders take and report sectors as `u64`, and a refused fill says why
  with `FillError`. A format too large for the widths it writes is `FormatError::TooLarge`.
- Breaking: `WritableVolume` no longer dereferences to `Volume`; `as_volume` and
  `as_volume_mut` lend it.
- A `Volume`, `WritableVolume`, `Directory` and `FileReader` take the length of their sector
  buffer as a const parameter, 4,096 bytes by default. `open_with_buffer` opens with 512, 1,024 or
  2,048, and refuses a device whose sectors are longer with `ReadError::SectorLargerThanBuffer`.
- The crate page carries the README's example, every example shows its errors with `?`, and every
  fallible public function has an `# Errors` section, which clippy now asks for.
- The README states the sizes the types have, and a test holds each figure to `size_of`.
- Six value types derive `Hash`, and the manifest names its documentation.

## 0.1.0-pre.3

- Two match arms that could never run are gone. A new entry now carries its name as one value, either a plain short name or a long name, and the table write that links a new cluster computes each entry's place once for every copy of the table. New tests cover the full-volume and corrupt-entry refusals, names outside Latin-1, and the hidden-sector count of a format.
- The README and the rustdoc now answer the nine gaps that the second stranger run found. They name
  the flag that runs the `embedded-io` tests, the dependency a caller adds, and the tests that pass
  without comparing when mtools is absent. They also cover `Partition::restrict_device` and the long name that a lower-case 8.3 name keeps.
  They state the unit of the image example, the count of Kani harnesses and their memory, and the
  bytes past a file's end. They state the path separators of `lookup`, and they document the
  refusal of a name that ends in a dot or a space.
- The README applies a falsification patch with `--no-backup-if-mismatch`, so a patch that applies with an offset leaves no `.orig` file in the source tree.
- The README reports how long a release build takes to fill a 32-bit File Allocation Table directory to its 65,536-entry limit.
- docs.rs builds with every feature on and marks the `embedded-io-07` trait impls with their
  feature. A test holds the README's harness count to the number of `#[kani::proof]` items.

## 0.1.0-pre.2

- The `embedded-io-07` feature, off by default, implements the `embedded-io` 0.7 traits for the
  file reader and writer, so a file plugs into crates written against them. The default build
  keeps no dependency. A seek before the start of a file fails with the new `SeekBeforeStart`
  variant of `FileReadError` and `WriteError`.
- A volume under 65525 clusters opens as a 32-bit File Allocation Table (FAT32) volume when the
  caller asks. The default cluster size follows the volume's size.
- A name that is 8.3 but for its case has its plain alias, and the reader honors the lower-case
  flags of a short name.
- The writer zeroes a directory's new cluster before it links it, and a slot lookup starts where
  the last one in the chain ended.
- A host example runs over an image file.

## 0.1.0-pre.1

- First release. Reads, writes and formats File Allocation Table (FAT) volumes in all three widths
  (FAT12, FAT16 and FAT32) as pure computation, with `no_std` and no allocator. The caller
  implements the sector device.
- The reader covers the boot sector, cluster chains, directories, long file names, paths and file
  bytes. The writer creates, writes, extends, truncates, renames and removes files and directories,
  sets attributes and the label, and marks the volume clean.
- `FormatPlan` lays out an empty volume of any size and sector size, and `VolumeLayout` lays out a
  FAT32 Extensible Firmware Interface (EFI) system partition that holds one boot file.
  `MasterBootRecord` opens a volume inside a partitioned disk for reading.
- Kani proves every parser of bytes read from a disk, with a falsification patch for each harness.
  The tests compare against an independent library and two images a macOS host wrote, and two
  cargo-fuzz targets run weekly.
- The crate forbids `unsafe` code.

## BUGS

- The template's lag check guards the CI pin, not this file; a change that matters to a user of
  the crate belongs in a release line here.
