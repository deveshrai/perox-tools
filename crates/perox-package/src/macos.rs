//! macOS: `Name.app` (the executable, Info.plist, the icon) and a `.dmg` to drag it from.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::AppInfo;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The bundle's Info.plist.
pub fn info_plist(info: &AppInfo, has_icon: bool) -> String {
    let icon = if has_icon {
        "  <key>CFBundleIconFile</key><string>AppIcon</string>\n"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>{id}</string>
  <key>CFBundleExecutable</key><string>{exe}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>{publisher}</string>
{icon}</dict>
</plist>
"#,
        name = esc(&info.display_name),
        id = esc(&info.identifier),
        exe = esc(&info.bin),
        version = esc(&info.version),
        publisher = esc(&info.publisher),
    )
}

/// Builds `out/Name.app`.
pub fn app_bundle(
    info: &AppInfo,
    out: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<PathBuf, String> {
    let app = out.join(format!("{}.app", info.display_name));
    let _ = std::fs::remove_dir_all(&app);
    let contents = app.join("Contents");
    let io = |e: std::io::Error| format!("Couldn't write the app bundle: {e}");
    std::fs::create_dir_all(contents.join("MacOS")).map_err(io)?;
    std::fs::create_dir_all(contents.join("Resources")).map_err(io)?;
    let exe = contents.join("MacOS").join(&info.bin);
    std::fs::copy(&info.binary, &exe).map_err(io)?;
    let has_icon = match &info.icon {
        Some(png) => match icns(png, &contents.join("Resources/AppIcon.icns")) {
            Ok(()) => true,
            Err(e) => {
                log(&format!("No icon: {e}"));
                false
            }
        },
        None => false,
    };
    std::fs::write(contents.join("Info.plist"), info_plist(info, has_icon)).map_err(io)?;
    log(&format!("Made {}", app.display()));
    Ok(app)
}

/// A square PNG → `.icns`, with the sizes macOS wants (`sips` and `iconutil`).
fn icns(png: &Path, out: &Path) -> Result<(), String> {
    let set = out.with_extension("iconset");
    let _ = std::fs::remove_dir_all(&set);
    std::fs::create_dir_all(&set).map_err(|e| e.to_string())?;
    for size in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let px = size * scale;
            let name = if scale == 1 {
                format!("icon_{size}x{size}.png")
            } else {
                format!("icon_{size}x{size}@2x.png")
            };
            let ok = Command::new("sips")
                .args(["-z", &px.to_string(), &px.to_string()])
                .arg(png)
                .arg("--out")
                .arg(set.join(name))
                .output()
                .is_ok_and(|o| o.status.success());
            if !ok {
                return Err(format!("sips couldn't resize {}", png.display()));
            }
        }
    }
    let ok = Command::new("iconutil")
        .args(["-c", "icns"])
        .arg(&set)
        .arg("-o")
        .arg(out)
        .output()
        .is_ok_and(|o| o.status.success());
    let _ = std::fs::remove_dir_all(&set);
    if ok {
        Ok(())
    } else {
        Err("iconutil couldn't make the .icns".into())
    }
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let o = cmd
        .output()
        .map_err(|e| format!("Couldn't run {what}: {e}"))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// Signs the bundle with `PEROX_MACOS_SIGN_IDENTITY` if it's set (hardened runtime, so it
/// can be notarized); otherwise ad hoc, which Apple Silicon needs to run it at all.
pub fn sign(app: &Path, log: &mut dyn FnMut(&str)) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    match std::env::var("PEROX_MACOS_SIGN_IDENTITY")
        .ok()
        .filter(|s| !s.is_empty())
    {
        Some(id) => {
            run(
                Command::new("codesign")
                    .args([
                        "--force",
                        "--deep",
                        "--options",
                        "runtime",
                        "--timestamp",
                        "--sign",
                    ])
                    .arg(&id)
                    .arg(app),
                "codesign",
            )?;
            log(&format!("Signed as {id}"));
        }
        None => {
            run(
                Command::new("codesign")
                    .args(["--force", "--deep", "--sign", "-"])
                    .arg(app),
                "codesign",
            )?;
            log(
                "Signed ad hoc (no PEROX_MACOS_SIGN_IDENTITY): people will be asked to allow it the first time",
            );
        }
    }
    Ok(())
}

/// A compressed disk image with the app and a link to Applications; notarized and
/// stapled when `PEROX_MACOS_NOTARY_PROFILE` is set.
pub fn dmg(
    info: &AppInfo,
    app: &Path,
    out: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<PathBuf, String> {
    let stage = out.join("dmg-stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let name = app.file_name().ok_or("no app name")?;
    run(
        Command::new("cp").arg("-R").arg(app).arg(stage.join(name)),
        "cp",
    )?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", stage.join("Applications"))
        .map_err(|e| e.to_string())?;
    let file = out.join(format!("{}.dmg", info.file_stem()));
    let _ = std::fs::remove_file(&file);
    run(
        Command::new("hdiutil")
            .args(["create", "-quiet", "-volname"])
            .arg(&info.display_name)
            .arg("-srcfolder")
            .arg(&stage)
            .args(["-ov", "-format", "UDZO"])
            .arg(&file),
        "hdiutil",
    )?;
    let _ = std::fs::remove_dir_all(&stage);
    if let Some(profile) = std::env::var("PEROX_MACOS_NOTARY_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
    {
        log("Notarizing (this takes a few minutes)…");
        run(
            Command::new("xcrun")
                .args(["notarytool", "submit", "--wait", "--keychain-profile"])
                .arg(&profile)
                .arg(&file),
            "notarytool",
        )?;
        run(
            Command::new("xcrun").args(["stapler", "staple"]).arg(&file),
            "stapler",
        )?;
        log("Notarized and stapled");
    }
    log(&format!("Made {}", file.display()));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_names_the_app() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-mac-{}", std::process::id()));
        let i = crate::info::sample(&dir);
        let p = info_plist(&i, false);
        assert!(p.contains("<key>CFBundleExecutable</key><string>stock-dashboard</string>"));
        assert!(p.contains("<string>Stock Dashboard</string>"));
        assert!(p.contains("<string>1.2.3</string>"));
        assert!(!p.contains("CFBundleIconFile"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn makes_a_bundle_and_a_disk_image() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-mac2-{}", std::process::id()));
        let i = crate::info::sample(&dir);
        let mut lines = Vec::new();
        let made = crate::package(
            &i,
            &dir.join("out"),
            &[crate::Format::MacApp, crate::Format::Dmg],
            &mut |l| lines.push(l.to_owned()),
        )
        .unwrap();
        assert!(made[0].join("Contents/MacOS/stock-dashboard").is_file());
        assert!(made[0].join("Contents/Info.plist").is_file());
        assert!(
            made[1].ends_with("stock-dashboard-1.2.3.dmg") && made[1].is_file(),
            "{lines:?}"
        );
    }
}
