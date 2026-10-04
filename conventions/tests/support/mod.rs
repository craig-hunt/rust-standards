//! Finding and parsing this repository's own sources and manifests.
//!
//! A module directory rather than a sibling file, because cargo compiles every
//! top-level file under `tests/` as its own test binary and this one holds no
//! tests.
//!
//! Dead code is allowed here for a reason that is structural rather than
//! careless: cargo compiles this module separately into each test binary, so a
//! helper two rules share looks unused to whichever binary does not call it.
//!
//! Rules about source text need the text. A compiled artifact has already folded
//! every literal into a constant with no record of where it was written, so the
//! only way to ask where a literal sits is to read the file.

// Panicking is how this module reports a repository it cannot read: a source
// that does not parse means the build already failed, and there is nothing
// useful to recover to. Scoped here, so production code still cannot reach for
// it.
#![allow(clippy::panic, dead_code)]

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// The crates the workspace builds, in dependency order.
pub(crate) const CRATES: [&str; 5] = [
    "domain",
    "application",
    "infrastructure",
    "web",
    "conventions",
];

const CARGO_MANIFEST: &str = "Cargo.toml";
const RUST_EXTENSION: &str = "rs";
const PARENT: &str = "..";
const WINDOWS_SEPARATOR: char = '\\';
const PORTABLE_SEPARATOR: &str = "/";
const MAX_DEPTH_UPWARD: usize = 5;

/// The repository root, located by walking up until every crate is present.
///
/// Cargo sets `CARGO_MANIFEST_DIR` to this crate's directory, not the
/// workspace's, and a developer may run from either. Walking up until the shape
/// matches answers correctly from anywhere, and says so plainly if it cannot.
///
/// # Panics
///
/// Panics when no directory above the manifest holds every crate, which means
/// the test is running somewhere it cannot reason about.
#[must_use]
pub(crate) fn repository_root() -> PathBuf {
    let mut candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..=MAX_DEPTH_UPWARD {
        if CRATES
            .iter()
            .all(|crate_name| candidate.join(crate_name).is_dir())
        {
            return candidate;
        }
        candidate = candidate.join(PARENT);
        candidate = candidate.canonicalize().unwrap_or(candidate);
    }
    panic!(
        "no directory above {} holds every crate",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Every Rust source in the repository, main and test alike.
///
/// Includes test sources on purpose. A test that repeats a literal because the
/// code it checks reads a constant keeps passing after the constant changes.
#[must_use]
pub(crate) fn rust_sources() -> Vec<PathBuf> {
    let root = repository_root();
    let mut found: Vec<PathBuf> = CRATES
        .iter()
        .flat_map(|crate_name| sources_under(&root.join(crate_name)))
        .collect();
    found.sort();
    found
}

fn sources_under(directory: &Path) -> Vec<PathBuf> {
    WalkDir::new(directory)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|path| path.extension().is_some_and(|ext| ext == RUST_EXTENSION))
        .collect()
}

/// A path as a reader would cite it, relative to the repository root.
///
/// # Panics
///
/// Panics when the path is not under the root, which cannot happen for a path
/// this module produced.
#[must_use]
pub(crate) fn relative(path: &Path) -> String {
    path.strip_prefix(repository_root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace(WINDOWS_SEPARATOR, PORTABLE_SEPARATOR)
}

/// Parses one source file into a syntax tree.
///
/// # Panics
///
/// Panics when a file cannot be read or does not parse. Either means the
/// repository does not compile, which the build already established before these
/// tests ran, so there is nothing useful to recover to.
#[must_use]
pub(crate) fn parse(path: &Path) -> syn::File {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|failure| panic!("{} cannot be read: {failure}", relative(path)));
    syn::parse_file(&text)
        .unwrap_or_else(|failure| panic!("{} does not parse: {failure}", relative(path)))
}

/// One crate's manifest, parsed.
///
/// # Panics
///
/// Panics when the manifest is missing or malformed.
#[must_use]
pub(crate) fn manifest(crate_name: &str) -> toml::Table {
    let path = repository_root().join(crate_name).join(CARGO_MANIFEST);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|failure| panic!("{} cannot be read: {failure}", relative(&path)));
    text.parse::<toml::Table>()
        .unwrap_or_else(|failure| panic!("{} does not parse: {failure}", relative(&path)))
}

/// The names in one dependency table of a crate's manifest.
#[must_use]
pub(crate) fn dependencies(crate_name: &str, table: &str) -> Vec<String> {
    manifest(crate_name)
        .get(table)
        .and_then(toml::Value::as_table)
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}
