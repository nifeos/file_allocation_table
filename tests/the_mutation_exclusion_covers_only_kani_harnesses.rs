//! `.cargo/mutants.toml` keeps cargo-mutants out of every `src/**/verification.rs`. That is sound
//! only while each such file is a module declared under `#[cfg(kani)]`, which `cargo test` never
//! compiles, so no test could catch a mutant in it and Kani checks it instead. This test holds
//! the exclusion to that: a `verification.rs` that ordinary builds compile fails here.

use std::fs;
use std::path::{Path, PathBuf};

/// Every file named `verification.rs` under `dir`.
fn verification_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("the source directory lists") {
        let path = entry.expect("a directory entry reads").path();
        if path.is_dir() {
            verification_files(&path, out);
        } else if path
            .file_name()
            .is_some_and(|name| name == "verification.rs")
        {
            out.push(path);
        }
    }
}

/// The file that declares the module `dir`: `dir.rs` beside it, or `dir/mod.rs`, or for `src`
/// itself `src/lib.rs`.
fn declaring_file(dir: &Path, src: &Path) -> PathBuf {
    if dir == src {
        return src.join("lib.rs");
    }
    let beside = dir.with_extension("rs");
    if beside.is_file() {
        beside
    } else {
        dir.join("mod.rs")
    }
}

/// Whether `text` declares `mod verification;` (of any visibility) only with `#[cfg(kani)]` on
/// the line before, and declares it at least once.
fn declares_only_under_kani(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut found = false;
    for (n, line) in lines.iter().enumerate() {
        let declaration = line
            .split_once("mod verification")
            .is_some_and(|(before, after)| {
                after.starts_with(';')
                    && (before.is_empty() || before.starts_with("pub") && before.ends_with(' '))
            });
        if declaration {
            if n == 0 || lines[n - 1] != "#[cfg(kani)]" {
                return false;
            }
            found = true;
        }
    }
    found
}

#[test]
fn every_excluded_file_is_a_module_only_kani_compiles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The package leaves `.cargo` out, and where there is no exclusion there is nothing to hold.
    let Ok(config) = fs::read_to_string(root.join(".cargo/mutants.toml")) else {
        return;
    };
    assert!(
        config.contains(r#"exclude_globs = ["src/**/verification.rs"]"#),
        "the exclusion this test checks is not the one in .cargo/mutants.toml:\n{config}"
    );
    let src = root.join("src");
    let mut files = Vec::new();
    verification_files(&src, &mut files);
    assert!(!files.is_empty(), "no verification.rs under src");
    for file in files {
        let dir = file.parent().expect("a file has a directory");
        let declaring = declaring_file(dir, &src);
        let text = fs::read_to_string(&declaring)
            .unwrap_or_else(|error| panic!("{}: {error}", declaring.display()));
        assert!(
            declares_only_under_kani(&text),
            "{} is excluded from mutation, but {} does not declare it only under #[cfg(kani)]",
            file.display(),
            declaring.display()
        );
    }
}
