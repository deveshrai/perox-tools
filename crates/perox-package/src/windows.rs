//! Windows: a `.zip` to unpack anywhere, and an `.msi` installer when the WiX Toolset
//! (`wix`, version 4 or later: `dotnet tool install --global wix`) is installed.
//!
//! The installer puts the app in Program Files, adds it to the Start menu and to
//! Installed apps, and replaces an older version of the same app (by its identifier).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::AppInfo;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Whether `wix` runs.
pub fn wix_available() -> bool {
    Command::new("wix")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A GUID that's always the same for the same text (FNV-1a, 128 bit), so every version
/// of an app has the same upgrade code and replaces the one before.
pub fn stable_guid(text: &str) -> String {
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for b in text.bytes() {
        h ^= b as u128;
        h = h.wrapping_mul(0x0000000001000000000000000000013B);
    }
    let x = format!("{h:032X}");
    format!(
        "{}-{}-{}-{}-{}",
        &x[0..8],
        &x[8..12],
        &x[12..16],
        &x[16..20],
        &x[20..32]
    )
}

/// MSI versions are up to three numbers below 256.256.65536: `1.2.3-beta` → `1.2.3`.
pub fn msi_version(v: &str) -> String {
    let core = v.split(['-', '+']).next().unwrap_or("0");
    let mut parts: Vec<u32> = core.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(3, 0);
    format!(
        "{}.{}.{}",
        parts[0].min(255),
        parts[1].min(255),
        parts[2].min(65535)
    )
}

/// The WiX source for the installer. `exe` and `ico` are the files to put in it.
pub fn wxs(info: &AppInfo, exe: &Path, ico: Option<&Path>) -> String {
    let name = esc(&info.display_name);
    let publisher = if info.publisher.is_empty() {
        name.clone()
    } else {
        esc(&info.publisher)
    };
    let (icon, arp, shortcut_icon) = match ico {
        Some(i) => (
            format!(
                "    <Icon Id=\"AppIcon.ico\" SourceFile=\"{}\" />\n",
                esc(&i.to_string_lossy())
            ),
            "    <Property Id=\"ARPPRODUCTICON\" Value=\"AppIcon.ico\" />\n",
            " Icon=\"AppIcon.ico\"",
        ),
        None => (String::new(), "", ""),
    };
    format!(
        r#"<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
  <Package Name="{name}" Manufacturer="{publisher}" Version="{version}" UpgradeCode="{upgrade}" Scope="perMachine">
    <MajorUpgrade DowngradeErrorMessage="A newer version of [ProductName] is already installed." />
    <MediaTemplate EmbedCab="yes" />
{icon}{arp}    <StandardDirectory Id="ProgramFiles6432Folder">
      <Directory Id="INSTALLFOLDER" Name="{name}">
        <Component>
          <File Source="{exe}" KeyPath="yes">
            <Shortcut Name="{name}" Directory="ProgramMenuFolder" Advertise="yes"{shortcut_icon} />
          </File>
        </Component>
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder" />
  </Package>
</Wix>
"#,
        version = msi_version(&info.version),
        upgrade = stable_guid(&info.identifier),
        exe = esc(&exe.to_string_lossy()),
    )
}

/// The icon as a Windows `.ico` (16 to 256 px).
pub fn ico(png: &Path, out: &Path) -> Result<(), String> {
    let img =
        image::open(png).map_err(|e| format!("Couldn't read the icon {}: {e}", png.display()))?;
    let frames: Vec<image::codecs::ico::IcoFrame> = [16u32, 24, 32, 48, 64, 128, 256]
        .iter()
        .map(|&px| {
            let r = img
                .resize_exact(px, px, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            image::codecs::ico::IcoFrame::as_png(
                r.as_raw(),
                px,
                px,
                image::ExtendedColorType::Rgba8,
            )
        })
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let f = std::fs::File::create(out).map_err(|e| e.to_string())?;
    image::codecs::ico::IcoEncoder::new(f)
        .encode_images(&frames)
        .map_err(|e| e.to_string())
}

/// Makes `out/<name>-<version>.msi` with `wix build`.
pub fn msi(info: &AppInfo, out: &Path, log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    if !wix_available() {
        return Err(
            "The WiX Toolset isn't installed: `dotnet tool install --global wix` (or make a .zip instead)"
                .into(),
        );
    }
    let stage = out.join("msi-stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let exe = stage.join(format!("{}.exe", info.bin));
    std::fs::copy(&info.binary, &exe).map_err(|e| e.to_string())?;
    let icon = match &info.icon {
        Some(png) => {
            let i = stage.join("AppIcon.ico");
            match ico(png, &i) {
                Ok(()) => Some(i),
                Err(e) => {
                    log(&format!("No icon: {e}"));
                    None
                }
            }
        }
        None => None,
    };
    let source = stage.join("app.wxs");
    std::fs::write(&source, wxs(info, &exe, icon.as_deref())).map_err(|e| e.to_string())?;
    let file = out.join(format!("{}.msi", info.file_stem()));
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        _ => "x64",
    };
    let o = Command::new("wix")
        .args(["build", "-arch", arch, "-o"])
        .arg(&file)
        .arg(&source)
        .output()
        .map_err(|e| format!("Couldn't run wix: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "wix build failed: {}{}",
            String::from_utf8_lossy(&o.stdout).trim(),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    let _ = std::fs::remove_dir_all(&stage);
    // wix leaves a .wixpdb beside the .msi.
    let _ = std::fs::remove_file(file.with_extension("wixpdb"));
    log(&format!("Made {}", file.display()));
    Ok(file)
}

/// Makes `out/<name>-<version>-windows.zip`: a folder with the app (and its icon).
pub fn zip(info: &AppInfo, out: &Path, log: &mut dyn FnMut(&str)) -> Result<PathBuf, String> {
    let file = out.join(format!("{}-windows.zip", info.file_stem()));
    let f = std::fs::File::create(&file)
        .map_err(|e| format!("Couldn't write {}: {e}", file.display()))?;
    let mut z = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);
    let folder = &info.display_name;
    let err = |e: zip::result::ZipError| e.to_string();
    let exe = std::fs::read(&info.binary).map_err(|e| e.to_string())?;
    z.start_file(format!("{folder}/{}.exe", info.bin), opts)
        .map_err(err)?;
    z.write_all(&exe).map_err(|e| e.to_string())?;
    if let Some(png) = &info.icon {
        let tmp = out.join(format!("{}.ico.tmp", info.name));
        if ico(png, &tmp).is_ok() {
            z.start_file(format!("{folder}/{}.ico", info.bin), opts)
                .map_err(err)?;
            z.write_all(&std::fs::read(&tmp).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(tmp);
    }
    z.finish().map_err(err)?;
    log(&format!("Made {}", file.display()));
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_and_versions() {
        let a = stable_guid("dev.perox.stock-dashboard");
        assert_eq!(a, stable_guid("dev.perox.stock-dashboard"));
        assert_ne!(a, stable_guid("dev.perox.other"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert_eq!(msi_version("1.2.3"), "1.2.3");
        assert_eq!(msi_version("1.2.3-beta.1"), "1.2.3");
        assert_eq!(msi_version("2"), "2.0.0");
        assert_eq!(msi_version("300.1.1"), "255.1.1");
    }

    #[test]
    fn wix_source() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-wxs-{}", std::process::id()));
        let info = crate::info::sample(&dir);
        let s = wxs(&info, Path::new("C:\\stage\\stock-dashboard.exe"), None);
        assert!(s.contains("Name=\"Stock Dashboard\""));
        assert!(s.contains("Manufacturer=\"Example Ltd\""));
        assert!(s.contains("Version=\"1.2.3\""));
        assert!(s.contains(&stable_guid("dev.perox.stock-dashboard")));
        assert!(s.contains("Source=\"C:\\stage\\stock-dashboard.exe\""));
        assert!(!s.contains("ARPPRODUCTICON"));
        let s = wxs(&info, Path::new("a.exe"), Some(Path::new("i.ico")));
        assert!(s.contains("ARPPRODUCTICON"));
    }

    #[test]
    fn makes_a_zip_with_an_icon() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-zip-{}", std::process::id()));
        let mut info = crate::info::sample(&dir);
        let icon = dir.join("icon.png");
        image::RgbaImage::from_pixel(512, 512, image::Rgba([180, 180, 190, 255]))
            .save(&icon)
            .unwrap();
        info.icon = Some(icon);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let file = zip(&info, &out, &mut |_| {}).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().unwrap().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "Stock Dashboard/stock-dashboard.exe",
                "Stock Dashboard/stock-dashboard.ico"
            ]
        );
        let ico = image::load_from_memory_with_format(
            &{
                let mut v = Vec::new();
                std::io::Read::read_to_end(&mut z.by_index(1).unwrap(), &mut v).unwrap();
                v
            },
            image::ImageFormat::Ico,
        )
        .unwrap();
        assert_eq!(ico.width(), 256);
    }
}
