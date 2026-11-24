//! Finding the toolchain's programs. Order: the bundled Toolchain Set
//! (`PEROXIDE_TOOLCHAIN/bin`), then rustup's pinned toolchain, then `PATH`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// The Rust version of the Toolchain Set (toolchain-set/set.toml).
pub const RUST_VERSION: &str = "1.100.0";

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

fn locate(name: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PEROXIDE_TOOLCHAIN") {
        let p = PathBuf::from(dir).join("bin").join(exe(name));
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(out) = Command::new("rustup")
        .args(["which", "--toolchain", RUST_VERSION, name])
        .output()
        && out.status.success()
    {
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if p.exists() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe(name)))
        .find(|p| p.exists())
}

/// `cargo`, cached.
pub fn cargo() -> Option<PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| locate("cargo")).clone()
}

/// `rust-analyzer`, cached.
pub fn rust_analyzer() -> Option<PathBuf> {
    static P: OnceLock<Option<PathBuf>> = OnceLock::new();
    P.get_or_init(|| locate("rust-analyzer")).clone()
}

/// Environment for every tool the IDE starts (cargo, rust-analyzer, and through them
/// rustc and rustfmt), so they all use the pinned toolchain, never whatever the user's
/// rustup default happens to be.
pub fn env() -> Vec<(String, String)> {
    if let Some(dir) = std::env::var_os("PEROXIDE_TOOLCHAIN") {
        let bin = PathBuf::from(dir).join("bin");
        let mut paths = vec![bin];
        if let Some(old) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&old));
        }
        let path = std::env::join_paths(paths)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        return vec![("PATH".into(), path)];
    }
    if cargo().is_some_and(|c| c.components().any(|p| p.as_os_str() == ".rustup")) {
        return vec![("RUSTUP_TOOLCHAIN".into(), RUST_VERSION.into())];
    }
    Vec::new()
}

/// The folder holding the runtime crates new apps depend on (`perox-ui`,
/// `perox-slint`). Order: `PEROXIDE_SDK`; the install's `sdk` folder (next to the
/// executable, or `Contents/Resources/sdk` in a macOS bundle); this source tree's
/// `crates/` (development builds).
pub fn sdk_dir() -> Option<PathBuf> {
    let has_runtime = |d: &std::path::Path| d.join("perox-ui").join("Cargo.toml").exists();
    if let Some(d) = std::env::var_os("PEROXIDE_SDK").map(PathBuf::from)
        && has_runtime(&d)
    {
        return Some(d);
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from))
    {
        for d in [exe_dir.join("sdk"), exe_dir.join("../Resources/sdk")] {
            if has_runtime(&d) {
                return Some(crate::canonical(&d));
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    has_runtime(&dev).then(|| crate::canonical(&dev))
}

#[cfg(test)]
mod tests {
    #[test]
    fn pins_the_toolchain_under_rustup() {
        // On a developer machine cargo comes from ~/.rustup; the env must pin the version.
        if let Some(c) = super::cargo()
            && c.components().any(|p| p.as_os_str() == ".rustup")
        {
            assert!(
                super::env().contains(&("RUSTUP_TOOLCHAIN".into(), super::RUST_VERSION.into()))
            );
        }
    }
}
