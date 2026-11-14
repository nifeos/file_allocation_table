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

Last run: 2026-11-14

Result of that run: the stranger reached a passing build, 289 unit tests, 58 integration tests
(one more ignored, the macOS driver test) and 13 documentation tests, the same again with the
`embedded-io-07` feature on, and all 35 Kani proofs in about 18 minutes, and it built and ran a
program of its own against the crate, without reading any source file. Kani was already installed,
so the run again could not show whether the README's install steps work. It found no false claim.
It found nine gaps, the largest that the README names no flag for the `embedded-io` tests and never
says a caller needs its own `embedded-io` dependency; issue #49 proposes a fix for each. The full
record is in [the run appendix](stranger-test-run-2026-11-14.md), and the first run's in
[its own](stranger-test-run-2026-11-06.md).
