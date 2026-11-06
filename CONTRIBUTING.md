# Contributing

Issues first: the bug report and feature request forms are the front door, and blank issues are
off so a report lands in a shape somebody can act on. Changes go through pull requests; the
shared workflows are the gate, and a red job names its own fix.

## Disclosure

This project's code is written by AI agents working in parallel lanes, with one human reviewing
architecture and outcomes. That is disclosed rather than hidden, and it is the standing rule for
anything that lands here. Outside contributions
are welcome and reviewed the same way as agent-written ones: by what the diff does and what the
proofs say. A pull request written by an agent opens by saying so, in its first line, because the
account that pushes it is the maintainer's. A person outside the project who submits a change may use any tools they like
and does not need to say which.

## What the gates will ask of a change

- The crate stays `no_std`-friendly, dual licensed `MIT OR Apache-2.0`, and free of vocabulary
  from any other tree (the `standalone` job reads for it).
- A proof harness carries a falsification patch, or a reason of at least 200 characters that it
  cannot.
- A dependency new to the graph holds the pull request for the architect's ruling, and so does a
  breaking change against the last release. `script/record-ruling` records the answer.
