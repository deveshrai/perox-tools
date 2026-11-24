//! Finding a debug adapter (lldb-dap) and the toolchain's Rust support for LLDB.

use std::path::{Path, PathBuf};
use std::process::Command;

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

/// The debug adapter to use: `configured` (a setting) if given, else `PEROXIDE_DAP`,
/// else lldb-dap where the OS usually keeps it (ADR 0012).
pub fn find(configured: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = configured.filter(|p| p.is_file()) {
        return Some(p.to_owned());
    }
    if let Some(p) = std::env::var_os("PEROXIDE_DAP").map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    if cfg!(target_os = "macos")
        && let Ok(out) = Command::new("xcrun").args(["-f", "lldb-dap"]).output()
        && out.status.success()
    {
        let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if p.is_file() {
            return Some(p);
        }
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        dirs.push(PathBuf::from(r"C:\Program Files\LLVM\bin"));
    } else {
        dirs.push(PathBuf::from("/opt/homebrew/opt/llvm/bin"));
        dirs.push(PathBuf::from("/usr/local/opt/llvm/bin"));
        // Debian and Ubuntu: /usr/lib/llvm-18/bin, newest first.
        if let Ok(rd) = std::fs::read_dir("/usr/lib") {
            let mut llvm: Vec<(u32, PathBuf)> = rd
                .flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let v = n.strip_prefix("llvm-")?.parse().ok()?;
                    Some((v, e.path().join("bin")))
                })
                .collect();
            llvm.sort_by_key(|x| std::cmp::Reverse(x.0));
            dirs.extend(llvm.into_iter().map(|(_, p)| p));
        }
    }
    for name in ["lldb-dap", "lldb-vscode"] {
        if let Some(p) = dirs.iter().map(|d| d.join(exe(name))).find(|p| p.is_file()) {
            return Some(p);
        }
    }
    // `lldb-dap-18` and friends: take the newest.
    let mut versioned: Vec<(u32, PathBuf)> = Vec::new();
    for d in &dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let n = n.strip_suffix(".exe").unwrap_or(&n);
            if let Some(v) = n.strip_prefix("lldb-dap-").and_then(|v| v.parse().ok()) {
                versioned.push((v, e.path()));
            }
        }
    }
    versioned.sort_by_key(|x| std::cmp::Reverse(x.0));
    versioned.into_iter().map(|(_, p)| p).next()
}

/// Where to get a debugger, for the message shown when none is found.
pub fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install Xcode's command line tools (xcode-select --install), which include lldb-dap."
    } else if cfg!(windows) {
        "Install LLVM (winget install LLVM.LLVM), which includes lldb-dap, or set the adapter in Settings."
    } else {
        "Install LLDB (for example sudo apt install lldb), which includes lldb-dap."
    }
}

/// LLDB commands that make Rust values readable (`Vec`, `String`, `Option`, …) using the
/// formatters that ship with the toolchain in `sysroot`, and that map the standard
/// library's build paths to its source when `rust-src` is installed.
pub fn rust_init_commands(sysroot: &Path, commit_hash: Option<&str>) -> Vec<String> {
    let etc = sysroot.join("lib").join("rustlib").join("etc");
    let mut out = Vec::new();
    let lookup = etc.join("lldb_lookup.py");
    let commands = etc.join("lldb_commands");
    if lookup.is_file() {
        // Newer toolchains register everything from the module itself; older ones
        // also ship a command file.
        out.push(format!("command script import \"{}\"", lookup.display()));
        if commands.is_file() {
            out.push(format!("command source -s 0 \"{}\"", commands.display()));
        }
    }
    let src = sysroot.join("lib").join("rustlib").join("src").join("rust");
    if let Some(hash) = commit_hash
        && src.is_dir()
    {
        out.push(format!(
            "settings set target.source-map /rustc/{hash} \"{}\"",
            src.display()
        ));
    }
    out
}

/// The commit hash in `rustc -vV` output.
pub fn commit_hash(rustc_vv: &str) -> Option<&str> {
    rustc_vv
        .lines()
        .find_map(|l| l.strip_prefix("commit-hash: "))
        .map(str::trim)
        .filter(|h| *h != "unknown")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_commit_hash() {
        let vv = "rustc 1.100.0 (abc 2026-10-01)\nbinary: rustc\ncommit-hash: 0123abcd\nhost: x\n";
        assert_eq!(commit_hash(vv), Some("0123abcd"));
        assert_eq!(commit_hash("commit-hash: unknown\n"), None);
    }

    #[test]
    fn formatter_commands_need_the_files() {
        let dir = std::env::temp_dir().join(format!("perox-sysroot-{}", std::process::id()));
        let etc = dir.join("lib/rustlib/etc");
        std::fs::create_dir_all(&etc).unwrap();
        assert!(rust_init_commands(&dir, Some("h")).is_empty());
        std::fs::write(etc.join("lldb_lookup.py"), "").unwrap();
        assert_eq!(rust_init_commands(&dir, Some("h")).len(), 1);
        std::fs::write(etc.join("lldb_commands"), "").unwrap();
        std::fs::create_dir_all(dir.join("lib/rustlib/src/rust")).unwrap();
        let c = rust_init_commands(&dir, Some("h"));
        assert_eq!(c.len(), 3);
        assert!(c[0].starts_with("command script import"));
        assert!(c[2].starts_with("settings set target.source-map /rustc/h"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
