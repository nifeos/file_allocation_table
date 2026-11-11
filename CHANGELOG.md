# Changelog

Dates are UTC. One line per release, what changed and why, so a reader of the diff has the
sentence beside it.

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
