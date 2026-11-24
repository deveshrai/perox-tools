//! Packaging apps built with PeroxIDE for people to install.
//!
//! | OS | Packages |
//! |---|---|
//! | macOS | `Name.app`, and a `.dmg` with it and a link to Applications |
//! | Linux | a `.deb` (Debian, Ubuntu…), and an AppImage when `appimagetool` is installed |
//! | Windows | an `.msi` when WiX (`wix`) is installed, and a `.zip` |
//!
//! What goes in comes from the project's `Cargo.toml`: the package's name, version and
//! description, and optionally:
//!
//! ```toml
//! [package.metadata.peroxide]
//! display-name = "Stock Dashboard"     # default: the package name in words
//! identifier = "com.example.stocks"    # default: dev.perox.<name>
//! publisher = "Example Ltd"
//! icon = "ui/images/app-icon.png"      # a square PNG, 512 px or more
//! ```
//!
//! Signing uses the publisher's certificates when the environment names them:
//! `PEROX_MACOS_SIGN_IDENTITY` (a Developer ID Application identity) and
//! `PEROX_MACOS_NOTARY_PROFILE` (a `notarytool` keychain profile) on macOS. Without them
//! the packages are unsigned: they work, after the OS asks the person once.

pub mod deb;
pub mod info;
pub mod macos;
pub mod windows;

use std::path::{Path, PathBuf};

pub use info::AppInfo;

/// A kind of package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    MacApp,
    Dmg,
    Deb,
    AppImage,
    Msi,
    Zip,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::MacApp => "app",
            Format::Dmg => "dmg",
            Format::Deb => "deb",
            Format::AppImage => "appimage",
            Format::Msi => "msi",
            Format::Zip => "zip",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "app" => Format::MacApp,
            "dmg" => Format::Dmg,
            "deb" => Format::Deb,
            "appimage" => Format::AppImage,
            "msi" => Format::Msi,
            "zip" => Format::Zip,
            _ => return None,
        })
    }
}

/// What can be made on this computer, by default: each OS packages for itself.
pub fn host_formats() -> Vec<Format> {
    if cfg!(target_os = "macos") {
        vec![Format::MacApp, Format::Dmg]
    } else if cfg!(target_os = "windows") {
        let mut f = vec![Format::Zip];
        if windows::wix_available() {
            f.insert(0, Format::Msi);
        }
        f
    } else {
        let mut f = vec![Format::Deb];
        if deb::appimagetool().is_some() {
            f.push(Format::AppImage);
        }
        f
    }
}

/// Makes `formats` for `info` in `out` (created if needed), reporting each step to
/// `log`. Returns the files made.
pub fn package(
    info: &AppInfo,
    out: &Path,
    formats: &[Format],
    log: &mut dyn FnMut(&str),
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("Couldn't make {}: {e}", out.display()))?;
    if !info.binary.is_file() {
        return Err(format!(
            "There's no release build at {}: build it first (cargo build --release).",
            info.binary.display()
        ));
    }
    let mut made = Vec::new();
    let mut app: Option<PathBuf> = None;
    for f in formats {
        match f {
            Format::MacApp | Format::Dmg => {
                let bundle = match &app {
                    Some(a) => a.clone(),
                    None => {
                        let a = macos::app_bundle(info, out, log)?;
                        macos::sign(&a, log)?;
                        app = Some(a.clone());
                        a
                    }
                };
                if *f == Format::MacApp {
                    made.push(bundle);
                } else {
                    made.push(macos::dmg(info, &bundle, out, log)?);
                }
            }
            Format::Deb => made.push(deb::deb(info, out, log)?),
            Format::AppImage => made.push(deb::appimage(info, out, log)?),
            Format::Msi => made.push(windows::msi(info, out, log)?),
            Format::Zip => made.push(windows::zip(info, out, log)?),
        }
    }
    Ok(made)
}
