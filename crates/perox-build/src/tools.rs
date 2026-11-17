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
