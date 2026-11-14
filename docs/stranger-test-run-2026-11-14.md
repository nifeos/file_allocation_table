# Stranger run, 2026-11-14

The second run of this repository's stranger test, on the release candidate of this File
Allocation Table (FAT) crate. This page records what the stranger did and what it found. The fixes
it calls for are proposed in issue #49 and are not part of this record.

## How it was run

- A fresh agent session, model Opus. Its instructions said that any project-instruction or memory
  text in its context concerned other projects and that it should ignore that text.
- Its working directory held one entry, a fresh clone of `main` from GitHub at `2215cbc`. Its
  instructions confined it to that directory and forbade reading anything outside it.
- The operator withheld the answer key by deleting `docs/stranger-test.md` and the first run's
  record, and committing the deletion in the clone.
- Its guides were the README and the rustdoc (from `cargo doc`), and it had to report as a finding
  every other file it read. It read none, and opened nothing under `fuzz/`.
- Task text, in short: get the crate building and its tests and proofs passing from the README;
  then, in a separate crate with a path dependency on the clone, write a program that formats an
  in-memory volume, writes files and directories, reads them back and tries what the docs suggest,
  the `embedded-io` feature included; check its images with `fsck.fat -n`; then write up what the
  crate is. It was asked to say whenever it guessed and to write down every question it would have
  asked a person.
- Every cargo command ran under a 12 GiB memory cap. The session ran for about 20 minutes, most of
  it the proof run.

## Result

| Measure | Outcome |
|---|---|
| Build | passed, `cargo build`, no warnings |
| Tests | passed: 289 unit tests, 58 integration tests (1 ignored, the macOS driver test), 13 doctests (1 ignored, the README's `embedded-io` example, which is `rust,ignore`), 19 seconds |
| Tests with the feature | passed, `cargo test --features embedded-io-07` (the flag a guess): 295 unit, 65 integration, 13 doctests |
| Documentation | `cargo doc --no-deps --all-features` passed, no warnings |
| Proofs | passed: `cargo kani`, 35 of 35 harnesses, 992 seconds of verification in 17.5 minutes, 4.6 gigabytes of peak memory; the slowest took 265, 130 and 93 seconds |
| Its own program | compiled and ran on FAT12, FAT16 and FAT32: nested directories with long names, 100,000 bytes written in pieces and read back, seek, `set_len` longer and shorter, numbered aliases, every refusal it tried, rename and move, removal, label, times, a full root and a full volume, the clean bit and the FAT32 hints, the `embedded-io` traits, and a partition it built by hand |
| `fsck.fat -n` on its images | clean on every image it closed with `mark_clean`; the two it dropped without it showed the dirty bit and, on FAT32, the stale free-cluster hint, as the docs say |
| Files read beyond README and rustdoc | none |
| Installed by the stranger | nothing: Kani 0.68.0, `fsck.fat` and `mkfs.fat` were already present, mtools absent, so the run did not test the Kani install path |

Kani printed the same two warnings as the first run and still reported success: `use of an
unstable feature` (`#![feature(register_tool)]`), and `Found the following unsupported
constructs: caller_location (1), foreign function (1)`.

## Questions, from its write-up

1. Which command runs the `embedded-io` tests: `--features embedded-io-07` or `--all-features`?
2. Does the crate re-export `embedded_io`, or must a caller depend on `embedded-io = "0.7"`?
3. Which features does docs.rs build with?
4. Should a test whose tool is absent (mtools, `mkfs.fat`) report as passed, and how does a user
   tell what ran?
5. The BUGS list says a newly allocated cluster holds whatever the device held, and `set_len` says
   a longer length reads as zeros. When does a caller see stale bytes?
6. Are a leading `/` in a lookup path, and the refusal of a name ending in a dot, both intended?
7. How much memory and how many cores does `cargo kani` need?
8. Does `cargo kani setup` install a pinned nightly toolchain?
9. Is the directory limit of 65,536 entries reachable in reasonable time?

## Findings

Issue #49 carries a proposed fix for each, and says which need a decision rather than a wording
change.

| Finding | Kind |
|---|---|
| "The tests run this example with the feature on" names no flag, and plain `cargo test` runs none of that file's tests. | omission |
| The `embedded-io` example needs the caller's own `embedded-io = "0.7"`; the README gives only the feature line. | omission |
| Without mtools, its comparison tests report as passed, with no visible sign that they compared nothing. | assumption |
| The README never names `Partition::restrict_device`; only the rustdoc does. | omission |
| "(`readme.txt` is `README.TXT`)" reads as a short entry only; the crate also keeps `readme.txt` as a long name. | omission |
| The unit of `64` in `format disk.img 64` is unstated, and the example writes its files into the current directory. | omission |
| The proof run's length is right, but the harness count (35) and memory (4.6 gigabytes) are missing. | omission |
| The BUGS line on stale cluster bytes reads as contradicting `set_len`, which writes zeros. | apparent contradiction |
| A leading `/` (and `\`) in a path is accepted, and a name ending in a dot is refused; neither is documented. | omission |

It checked four claims and found them true: `cargo test` finishing well under a minute, the
default format table, `SeekBeforeStart` and the seek that stops at the end, and the stale FAT32
free-cluster hint that `fsck.fat` reports.

## Confident wrong answers

It took "The tests run this example with the feature on" to mean that plain `cargo test` covered
the `embedded-io` code, and counted its first test run as covering it, until it saw that file
report 0 tests.

## What the stranger said the crate is

A `no_std` library for FAT12, FAT16 and FAT32 volumes with no allocator, which does no input or
output itself and works through a sector-device trait the caller implements. It reads every field
as hostile; writes files and directories with long names and numbered aliases; formats by the
specification's size tables, lays out an Extensible Firmware Interface (EFI) system partition and reads a simple Master Boot
Record; stamps writes from a caller's clock taken as UTC; and offers the `embedded-io` 0.7 traits
behind a feature. Bounded Kani proofs cover the parsing and table logic, tests compare it against
rust-fatfs and `fsck.fat`, and it is neither crash-atomic nor cached.
