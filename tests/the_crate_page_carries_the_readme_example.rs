//! **The crate page's example is the README's first example**, line for line, so the two cannot
//! drift. Both come from `examples/memory.rs`: the crate page includes the file, and the README's
//! copy, which runs as a documentation test through `ReadmeExample`, must equal it.

#[test]
fn the_crate_page_carries_the_readme_example() {
    let source = include_str!("../src/lib.rs");
    assert!(
        source.contains("//! ```\n#![doc = include_str!(\"../examples/memory.rs\")]\n//! ```\n"),
        "the crate page's Examples section includes examples/memory.rs"
    );
    let example = include_str!("../examples/memory.rs");
    let readme = include_str!("../README.md");
    assert!(
        readme.contains(&format!("```rust\n{example}```\n")),
        "the README's first example differs from examples/memory.rs:\n{example}"
    );
}
