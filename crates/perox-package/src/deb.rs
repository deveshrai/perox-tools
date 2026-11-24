//! Linux: a `.deb`, made here in Rust (no `dpkg-deb` needed), and an AppImage when
//! `appimagetool` is installed.
//!
//! Both install the same things: the executable, a `.desktop` file so the app shows in
//! the desktop's app list, and the icon.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::AppInfo;

/// The Debian name for the architecture being built for.
pub fn deb_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "i386",
        "arm" => "armhf",
        "riscv64" => "riscv64",
        other => other,
    }
}

/// Debian package names are lower case letters, digits and `+-.`.
pub fn package_name(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "+-.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The `.desktop` file. `exec` is the command line that starts the app.
pub fn desktop_entry(info: &AppInfo, exec: &str, icon: Option<&str>) -> String {
    let mut s = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={exec}\nTerminal=false\nCategories=Utility;\n",
        info.display_name
    );
    if !info.description.is_empty() {
        s.push_str(&format!("Comment={}\n", info.description));
    }
    if let Some(i) = icon {
        s.push_str(&format!("Icon={i}\n"));
    }
    s
}

/// The control file. `installed_kb` is the unpacked size.
pub fn control(info: &AppInfo, installed_kb: u64) -> String {
    let maintainer = if info.publisher.is_empty() {
        "Unknown <unknown@localhost>".to_owned()
    } else if info.publisher.contains('<') {
        info.publisher.clone()
    } else {
        format!("{} <unknown@localhost>", info.publisher)
    };
    let summary = if info.description.is_empty() {
        info.display_name.clone()
    } else {
        info.description
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    format!(
        "Package: {}\nVersion: {}\nArchitecture: {}\nMaintainer: {maintainer}\nInstalled-Size: {installed_kb}\nSection: misc\nPriority: optional\nDepends: libc6, libfontconfig1, libxkbcommon0\nRecommends: libxkbcommon-x11-0, libwayland-client0, libgl1\nDescription: {summary}\n",
        package_name(&info.name),
        info.version,
        deb_arch(),
    )
}

/// The icon at a size, as PNG bytes.
fn icon_png(src: &Path, px: u32) -> Result<Vec<u8>, String> {
    let img =
        image::open(src).map_err(|e| format!("Couldn't read the icon {}: {e}", src.display()))?;
    let img = img.resize_exact(px, px, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// A file going into a package: where, what, and whether it's executable.
struct Entry {
    path: String,
    data: Vec<u8>,
    exec: bool,
}

fn installed_files(info: &AppInfo, log: &mut dyn FnMut(&str)) -> Result<Vec<Entry>, String> {
    let bin = std::fs::read(&info.binary)
        .map_err(|e| format!("Couldn't read {}: {e}", info.binary.display()))?;
    let mut files = vec![Entry {
        path: format!("usr/bin/{}", info.bin),
        data: bin,
        exec: true,
    }];
    let mut icon = None;
    if let Some(src) = &info.icon {
        match icon_png(src, 256) {
            Ok(png) => {
                files.push(Entry {
                    path: format!(
                        "usr/share/icons/hicolor/256x256/apps/{}.png",
                        info.identifier
                    ),
                    data: png,
                    exec: false,
                });
                icon = Some(info.identifier.as_str());
            }
            Err(e) => log(&format!("No icon: {e}")),
        }
    }
    files.push(Entry {
        path: format!("usr/share/applications/{}.desktop", info.identifier),
        data: desktop_entry(info, &info.bin, icon).into_bytes(),
        exec: false,
    });
    Ok(files)
}

/// A gzipped tar of `files`, with their parent folders, owned by root.
fn tar_gz(files: &[Entry]) -> Result<Vec<u8>, String> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    let mut dirs: Vec<String> = Vec::new();
    for f in files {
        let mut acc = String::from(".");
        for part in f
            .path
            .split('/')
            .collect::<Vec<_>>()
            .split_last()
            .map(|x| x.1)
            .unwrap_or(&[])
        {
            acc = format!("{acc}/{part}");
            if !dirs.contains(&acc) {
                dirs.push(acc.clone());
            }
        }
    }
    let header = |size: u64, mode: u32, kind: tar::EntryType| {
        let mut h = tar::Header::new_gnu();
        h.set_size(size);
        h.set_mode(mode);
        h.set_uid(0);
        h.set_gid(0);
        h.set_mtime(0);
        h.set_entry_type(kind);
        h
    };
    let io = |e: std::io::Error| e.to_string();
    let mut h = header(0, 0o755, tar::EntryType::Directory);
    tar.append_data(&mut h, "./", std::io::empty())
        .map_err(io)?;
    for d in &dirs {
        let mut h = header(0, 0o755, tar::EntryType::Directory);
        tar.append_data(&mut h, format!("{d}/"), std::io::empty())
            .map_err(io)?;
    }
    for f in files {
        let mut h = header(
            f.data.len() as u64,
            if f.exec { 0o755 } else { 0o644 },
            tar::EntryType::Regular,
        );
        tar.append_data(&mut h, format!("./{}", f.path), f.data.as_slice())
            .map_err(io)?;
    }
    tar.into_inner().map_err(io)?.finish().map_err(io)
}

/// An `ar` archive (what a `.deb` is) of `(name, data)` members.
fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            name,
            0,
            0,
            0,
            "100644",
            data.len()
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// Makes `out/<name>_<version>_<arch>.deb`.
pub fn deb(info: &AppInfo, out: &Path, log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    let files = installed_files(info, log)?;
    let size: u64 = files.iter().map(|f| f.data.len() as u64).sum();
    let control_tar = tar_gz(&[Entry {
        path: "control".into(),
        data: control(info, size.div_ceil(1024)).into_bytes(),
        exec: false,
    }])?;
    let data_tar = tar_gz(&files)?;
    let file = out.join(format!(
        "{}_{}_{}.deb",
        package_name(&info.name),
        info.version,
        deb_arch()
    ));
    let bytes = ar(&[
        ("debian-binary", b"2.0\n"),
        ("control.tar.gz", &control_tar),
        ("data.tar.gz", &data_tar),
    ]);
    std::fs::File::create(&file)
        .and_then(|mut f| f.write_all(&bytes))
        .map_err(|e| format!("Couldn't write {}: {e}", file.display()))?;
    log(&format!("Made {}", file.display()));
    Ok(file)
}

/// `appimagetool`, if it's on the PATH (or named by `PEROX_APPIMAGETOOL`).
pub fn appimagetool() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PEROX_APPIMAGETOOL").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|d| {
            [
                "appimagetool",
                "appimagetool-x86_64.AppImage",
                "appimagetool-aarch64.AppImage",
            ]
            .map(|n| d.join(n))
        })
        .find(|p| p.is_file())
}

/// Makes `out/<Name>-<version>-<arch>.AppImage` with `appimagetool`.
pub fn appimage(info: &AppInfo, out: &Path, log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    let tool = appimagetool().ok_or(
        "appimagetool isn't installed: get it from github.com/AppImage/appimagetool, or set PEROX_APPIMAGETOOL",
    )?;
    let dir = out.join(format!("{}.AppDir", info.name));
    let _ = std::fs::remove_dir_all(&dir);
    let io = |e: std::io::Error| format!("Couldn't make the AppDir: {e}");
    for f in installed_files(info, log)? {
        let p = dir.join(&f.path);
        std::fs::create_dir_all(p.parent().unwrap_or(&dir)).map_err(io)?;
        std::fs::write(&p, &f.data).map_err(io)?;
        #[cfg(unix)]
        if f.exec {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
        }
    }
    // appimagetool wants the .desktop file and icon at the top, and an AppRun.
    std::fs::write(
        dir.join(format!("{}.desktop", info.identifier)),
        desktop_entry(info, &info.bin, Some(&info.identifier)),
    )
    .map_err(io)?;
    let icon = dir.join(format!(
        "usr/share/icons/hicolor/256x256/apps/{}.png",
        info.identifier
    ));
    if icon.is_file() {
        std::fs::copy(&icon, dir.join(format!("{}.png", info.identifier))).map_err(io)?;
    } else {
        // appimagetool refuses an AppDir without an icon: a plain one.
        let blank = image::RgbaImage::from_pixel(256, 256, image::Rgba([90, 90, 100, 255]));
        blank
            .save(dir.join(format!("{}.png", info.identifier)))
            .map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        let run = dir.join("AppRun");
        let _ = std::fs::remove_file(&run);
        std::os::unix::fs::symlink(format!("usr/bin/{}", info.bin), &run).map_err(io)?;
    }
    let arch = std::env::consts::ARCH;
    let file = out.join(format!(
        "{}-{}-{arch}.AppImage",
        info.display_name.replace(' ', "_"),
        info.version
    ));
    let o = Command::new(&tool)
        .arg(&dir)
        .arg(&file)
        .env("ARCH", arch)
        .output()
        .map_err(|e| format!("Couldn't run appimagetool: {e}"))?;
    let _ = std::fs::remove_dir_all(&dir);
    if !o.status.success() {
        return Err(format!(
            "appimagetool failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    log(&format!("Made {}", file.display()));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Reads back an `ar` archive.
    fn members(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert!(bytes.starts_with(b"!<arch>\n"));
        let mut i = 8;
        let mut out = Vec::new();
        while i < bytes.len() {
            let h = std::str::from_utf8(&bytes[i..i + 60]).unwrap();
            assert_eq!(&h[58..60], "`\n");
            let name = h[0..16].trim().to_owned();
            let size: usize = h[48..58].trim().parse().unwrap();
            out.push((name, bytes[i + 60..i + 60 + size].to_vec()));
            i += 60 + size + size % 2;
        }
        out
    }

    fn tar_names(gz: &[u8]) -> Vec<(String, u32)> {
        let mut t = tar::Archive::new(flate2::read::GzDecoder::new(gz));
        t.entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().unwrap().to_string_lossy().into_owned(),
                    e.header().mode().unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn makes_a_deb() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-deb-{}", std::process::id()));
        let mut info = crate::info::sample(&dir);
        let icon = dir.join("icon.png");
        image::RgbaImage::from_pixel(600, 600, image::Rgba([200, 80, 20, 255]))
            .save(&icon)
            .unwrap();
        info.icon = Some(icon);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let file = deb(&info, &out, &mut |_| {}).unwrap();
        assert_eq!(
            file.file_name().unwrap().to_string_lossy(),
            format!("stock-dashboard_1.2.3_{}.deb", deb_arch())
        );
        let m = members(&std::fs::read(&file).unwrap());
        let names: Vec<&str> = m.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["debian-binary", "control.tar.gz", "data.tar.gz"]);
        assert_eq!(m[0].1, b"2.0\n");

        let mut control = String::new();
        let mut t = tar::Archive::new(flate2::read::GzDecoder::new(m[1].1.as_slice()));
        for e in t.entries().unwrap() {
            let mut e = e.unwrap();
            if e.path().unwrap().ends_with("control") {
                e.read_to_string(&mut control).unwrap();
            }
        }
        assert!(control.contains("Package: stock-dashboard\n"));
        assert!(control.contains("Version: 1.2.3\n"));
        assert!(control.contains("Maintainer: Example Ltd <unknown@localhost>\n"));
        assert!(control.contains("Description: Stocks at a glance\n"));

        let data = tar_names(&m[2].1);
        assert!(
            data.contains(&("usr/bin/stock-dashboard".into(), 0o755)),
            "{data:?}"
        );
        assert!(data.contains(&(
            "usr/share/applications/dev.perox.stock-dashboard.desktop".into(),
            0o644
        )));
        assert!(
            data.iter()
                .any(|(p, _)| p.ends_with("256x256/apps/dev.perox.stock-dashboard.png"))
        );
        // Where dpkg is installed (the Linux CI), it must read the package too.
        if let Ok(o) = std::process::Command::new("dpkg-deb")
            .arg("--info")
            .arg(&file)
            .output()
        {
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            assert!(String::from_utf8_lossy(&o.stdout).contains("Package: stock-dashboard"));
        }
        assert!(
            data.iter()
                .any(|(p, m)| p.trim_end_matches('/') == "usr/share" && *m == 0o755),
            "{data:?}"
        );
    }

    #[test]
    fn names_and_desktop_entry() {
        assert_eq!(package_name("My_App"), "my-app");
        let dir = std::env::temp_dir().join(format!("perox-pkg-desk-{}", std::process::id()));
        let info = crate::info::sample(&dir);
        let d = desktop_entry(&info, "stock-dashboard", Some("dev.perox.stock-dashboard"));
        assert!(d.starts_with("[Desktop Entry]\n"));
        assert!(d.contains("Name=Stock Dashboard\n"));
        assert!(d.contains("Exec=stock-dashboard\n"));
        assert!(d.contains("Icon=dev.perox.stock-dashboard\n"));
        assert!(d.contains("Comment=Stocks at a glance\n"));
    }
}
