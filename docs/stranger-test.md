# The stranger cadence

The release's whole claim is that a stranger, given this repository and nothing else, can build
the crate and run its proofs from the README alone. A real agent runs that check on a
cadence, and the weekly `stranger cadence` job fails when the
last run is past 28 days.

What a run is:

1. Clone the repository fresh.
2. Follow the README and nothing else: build, test, and run the proofs.
3. Note every place the README lied, omitted, or assumed. Those are findings against the README,
   not against the stranger.
4. Record the date on the line below.

Last run: 2026-11-06

Result of that run: the stranger reached a passing build, 244 unit tests, 46 integration tests
(one more ignored, the macOS driver test) and 10 documentation tests, and all 31 Kani proofs in
about 18 minutes, and it built and ran a program of its own against the crate, without reading any
source file. It did not have to install Kani, because the machine already had it, so the run could
not show whether the README's install steps work. It found one false claim (an 8 MiB File
Allocation Table (FAT) format is FAT16, not FAT12) and one hazard the documentation never named
(an entry reused after a write through it reads stale data and can leak a cluster). The full
record is in [the run appendix](stranger-test-run-2026-11-06.md).
