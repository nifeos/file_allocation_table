# Security policy

A security report here is about shipped code: a way to make the crate corrupt memory it confines,
spend unbounded resources, or verify a property that does not hold. Report through GitHub's
private vulnerability reporting, which this repository has enabled. What you get back is a real
person reading the report, not a triage queue.

Two facts about how this code is written are worth knowing before you read it. Most of it was
written by AI agents working in parallel lanes, with one human reviewing architecture and
outcomes. And every property the crate claims is a Kani harness in the tree, with a falsification
patch that shows what turns it red; a report that contradicts a harness is also a bug in the
harness, and both halves are interesting.

## Supported versions

One line: the newest release. The shared workflows fail a call at an older template release, and
 Dependabot keeps the crate's own dependencies weekly, so a fix lands as a version bump you can
take.
