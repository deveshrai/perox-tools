//! Running cargo for the IDE. `tools` finds the toolchain's programs; `job` runs a cargo
//! command and turns its JSON output into diagnostics and Output-panel lines.

pub mod job;
pub mod testing;
pub mod tools;

pub use job::{Artifact, Diagnostic, Event, Job, Kind, Level};

/// `p` with links resolved and made absolute, without the `\\?\` prefix Windows adds
/// (git, cargo paths in manifests and file URIs don't take it). Unchanged if it doesn't
/// exist.
pub fn canonical(p: &std::path::Path) -> std::path::PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    match c.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => std::path::PathBuf::from(rest),
        _ => c,
    }
}

#[cfg(test)]
mod canonical_tests {
    #[test]
    fn no_verbatim_prefix() {
        let p = super::canonical(&std::env::temp_dir());
        assert!(!p.to_string_lossy().starts_with(r"\\?\"), "{}", p.display());
        assert!(p.is_absolute());
    }
}
