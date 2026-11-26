//! **The crate page's example is the README's first example**, line for line, so the two cannot
//! drift: the README's copy runs as a documentation test through `ReadmeExample` and the crate
//! page's through the crate's own documentation.

#[test]
fn the_crate_page_carries_the_readme_example() {
    let source = include_str!("../src/lib.rs");
    let (_, rest) = source
        .split_once("//! # Examples\n")
        .expect("the crate page has an Examples section");
    let (_, rest) = rest
        .split_once("//! ```\n")
        .expect("the section opens a code block");
    let (block, _) = rest.split_once("//! ```\n").expect("the code block closes");
    let example: String = block
        .lines()
        .map(|line| {
            let line = line
                .strip_prefix("//!")
                .expect("a crate documentation line");
            let line = line.strip_prefix(' ').unwrap_or(line);
            format!("{line}\n")
        })
        .collect();
    let readme = include_str!("../README.md");
    assert!(
        readme.contains(&format!("```rust\n{example}```\n")),
        "the crate page's example differs from the README's:\n{example}"
    );
}
