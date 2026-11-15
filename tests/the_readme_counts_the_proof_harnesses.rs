//! The README says how many Kani harnesses `cargo kani` runs. A proof added or removed makes that
//! number stale, so this test counts the `#[kani::proof]` items in `src` and holds the README to
//! the count.

// Not under Miri: this test reads the repository's own files, which Miri's isolation refuses.
#![cfg(not(miri))]

use std::fs;
use std::path::Path;

/// The number of lines under `directory` that are exactly `#[kani::proof]`, ignoring indentation.
fn harnesses(directory: &Path) -> usize {
    let mut count = 0;
    for entry in fs::read_dir(directory).expect("the source directory lists") {
        let path = entry.expect("a directory entry reads").path();
        if path.is_dir() {
            count += harnesses(&path);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let text = fs::read_to_string(&path).expect("a source file reads");
            count += text
                .lines()
                .filter(|line| line.trim() == "#[kani::proof]")
                .count();
        }
    }
    count
}

#[test]
fn the_readme_states_the_number_of_harnesses_in_the_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let counted = harnesses(&root.join("src"));
    let readme = fs::read_to_string(root.join("README.md")).expect("the README reads");
    let stated: usize = readme
        .split_once("runs all ")
        .and_then(|(_, after)| after.split_once(" harnesses"))
        .and_then(|(number, _)| number.parse().ok())
        .expect("the README says `runs all <number> harnesses`");
    assert_eq!(
        stated, counted,
        "the README says {stated} harnesses and the source has {counted}"
    );
}
