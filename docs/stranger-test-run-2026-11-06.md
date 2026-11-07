# Stranger run, 2026-11-06

The first run of this repository's stranger test, for the 0.1.0 release of this File Allocation
Table (FAT) crate. This page records what
the stranger did, what it found, and what the README and rustdoc now say as a result.

## How it was run

- A fresh agent session, model Opus. Its instructions said that any project-instruction or memory
  text in its context concerned other projects and that it should ignore that text.
- Its working directory held one entry, a fresh clone of `main` from GitHub at `c9e6302`. Its
  instructions confined it to that directory and forbade reading anything outside it.
- The operator withheld the answer key by deleting `docs/stranger-test.md` and committing the
  deletion in the clone.
- Its guides were the README and the rustdoc (from `cargo doc`, as docs.rs shows it), and it had
  to report as a finding every other file it read. It read none.
- Task text, in short: get the crate building and its tests and proofs passing from the README;
  then, in a separate crate with a path dependency on the clone (standing in for `cargo add`),
  write a small program that formats an in-memory volume, writes files, reads them back and tries
  what the docs suggest; then write up what the crate is. The task said the run measured it, and
  asked it to say so whenever it guessed and to write down every question it would have asked a
  person.
- The session ran for about 20 minutes, most of it the proof run.

## Result

| Measure | Outcome |
|---|---|
| Build | passed first time, `cargo build`, no warnings |
| Tests | passed: 244 unit tests, 46 integration tests (1 ignored, the macOS driver test), 10 doctests, 14 seconds |
| Documentation | `cargo doc --no-deps` passed |
| Proofs | passed: `cargo kani`, 31 of 31 harnesses, in 1055 seconds; the slowest took 291, 163 and 99 seconds |
| Its own program | compiled and ran: formats at seven sizes, a FAT16 volume with long names, a directory, a 20000-byte file read back byte for byte, seek, `set_len`, numbered aliases, refusals, attributes, rename, label, `mark_clean`, and a FAT32 volume |
| `fsck.fat -n` on its images | the FAT32 image clean; the FAT16 image one leaked cluster, traced to the stale-entry hazard below |
| Files read beyond README and rustdoc | none |
| Kani installed by the stranger | no. Kani 0.68.0 was already on the machine, so the run did not test the install path |

Kani printed two warnings and still reported success: `use of an unstable feature`
(`#![feature(register_tool)]`), and `Found the following unsupported constructs: caller_location
(1), foreign function (1)`.

## Questions, from its write-up

1. Is the 8 MiB example meant to be FAT12? If so, the format is wrong; if not, the README and the
   `FormatPlan` doc are wrong. What size gives FAT12?
2. After a write through a `FileWriter`, how does a caller get the updated `NamedEntry`? Must it
   always `lookup` again?
3. Should a second `open_file_writer` through a stale entry return `EntryChanged` rather than
   replace the file and leak a cluster?
4. How does a caller read a file's creation, write or access time?
5. Can a caller write to a volume inside a partition? `PartitionDevice` has no
   `WritableSectorDevice`.
6. Why does `FileOpenError` carry no reason, and why do the errors not implement `Display` or
   `core::error::Error`?
7. Is a cluster of one sector intended for an 80 MiB FAT32 volume?
8. Did the test suite run the `fsck.fat` and `mkfs.fat` comparisons? Nothing in the output says.
9. Is about 18 minutes normal for `cargo kani`, and can one harness run alone?
10. Are the Kani "unsupported constructs" warnings expected?

## Findings, and what changed

| Finding | Change |
|---|---|
| False claim: the README example and the `FormatPlan` doc both said an 8 MiB format is FAT12. It is FAT16, with 16223 clusters; FAT12 ends near 2 MiB. The doctests passed because neither checked the type. | Both now say FAT16. The README gives the FAT12 and FAT32 sizes at the default cluster size, and the `FormatPlan` doctest asserts the type. |
| A `NamedEntry` is a copy of the slot when it was read, and nothing said so. The entry `create_file` returns has no cluster and size 0; a reader from it reads 0 bytes, and a second writer from it replaces the file and leaks the old chain, with no error. `fsck.fat -n` reports the leak. The operator reproduced it. | A BUGS entry names the hazard and says to `lookup` again. The behavior itself is unchanged: whether `open_file_writer` should refuse a stale entry with `EntryChanged` is left open. |
| The README never named the build and test commands, and gave no idea how long the proofs take. | It now names `cargo build` and `cargo test`, gives 10 to 20 minutes for `cargo kani`, and names `cargo kani --harness`. |
| "A `MasterBootRecord` opens the volume inside a partitioned disk" left out that `PartitionDevice` only reads. | The README says so. |
| The crate overview pointed at `boot_sector.rs`, which a docs.rs reader cannot follow. | It links `FileAllocationTableType`. |
| The README says a write sets times, but no `NamedEntry` or `ShortEntry` method returns a time. | None; open. |
| `FileOpenError` carries no reason, and no error type implements `Display` or `core::error::Error`, so `?` into a boxed error does not work. | None; open. |
| The `PartitionDevice` example calls `open(Empty)` on a hidden `Empty`, and the `Volume` example asserts `chain(5)` is `[5, 6]` without saying why cluster 5. | None; open. |
| `remove_directory` and `remove_file` say "Remove the empty directory entry names", meaning the directory that `entry` names. Directories are named by a `u32` first cluster, with 0 for the root, which surprised the stranger. | None; open. |
| The format's default cluster size (one sector at every size tried, up to 300 MiB) is not stated, and differs from what `mkfs.fat` picks. | The README states it for the FAT12 and FAT32 sizes; whether it is the right default is open. |
| The test output does not say whether the `fsck.fat` and `mkfs.fat` comparisons ran. | None; open. |

## Confident wrong answers

The README told the stranger one false thing, and it acted on it: it believed 8 MiB was FAT12
until its own program printed FAT16. It also reused the entry `create_file` returned for a second
writer, which leaked a cluster that `fsck.fat` caught; nothing in the docs warned it.

## Limits of this run

- Kani, the Rust toolchain and dosfstools were already installed. A machine without them would
  test the install steps, and nobody has done that.
- The stranger was a fresh session but not a separate process with a stripped configuration. Its
  instructions asked it to ignore outside instructions and files, and nothing enforced that.
- The stranger did not run `cargo test -- --ignored`, since the host was not macOS.
- The operator is not the stranger. The operator wrote the README and rustdoc edits from the
  stranger's findings, and no second run has checked them.
