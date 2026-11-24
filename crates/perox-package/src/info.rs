//! What a package says about the app: from the project's `Cargo.toml`.

use std::path::{Path, PathBuf};

/// The app being packaged.
#[derive(Debug, Clone, PartialEq)]
pub struct AppInfo {
    /// The Cargo package name (`stock-dashboard`).
    pub name: String,
    /// The executable's name (`stock-dashboard`, `.exe` added on Windows).
    pub bin: String,
    /// What people see: `Stock Dashboard`.
    pub display_name: String,
    pub version: String,
    pub description: String,
    /// Reverse-DNS id (macOS bundle id, Linux desktop file name): `dev.perox.stock-dashboard`.
    pub identifier: String,
    pub publisher: String,
    /// A square PNG, if the project has one.
    pub icon: Option<PathBuf>,
    /// The release build to package.
    pub binary: PathBuf,
    /// The project folder.
    pub project: PathBuf,
}

/// `stock-dashboard` → `Stock Dashboard`.
pub fn words(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `[workspace.package].<key>` from the nearest workspace manifest above `project`.
fn workspace_value(project: &Path, key: &str) -> Option<String> {
    project.ancestors().skip(1).find_map(|dir| {
        let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        let doc: toml::Table = text.parse().ok()?;
        let ws = doc.get("workspace")?;
        Some(ws.get("package")?.get(key)?.as_str()?.to_owned())
    })
}

impl AppInfo {
    /// Reads `project/Cargo.toml`; the release build is looked for in `target_dir`
    /// (`target/release/<bin>`).
    pub fn load(project: &Path, target_dir: &Path) -> Result<Self, String> {
        let manifest = project.join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("Couldn't read {}: {e}", manifest.display()))?;
        let doc: toml::Table = text
            .parse()
            .map_err(|e| format!("{} isn't valid TOML: {e}", manifest.display()))?;
        let pkg = doc
            .get("package")
            .and_then(|p| p.as_table())
            .ok_or("Cargo.toml has no [package]: open the app's own folder")?;
        let s = |t: &toml::Table, k: &str| t.get(k).and_then(|v| v.as_str()).map(str::to_owned);
        let name = s(pkg, "name").ok_or("[package] has no name")?;
        // `version.workspace = true`: from the workspace's [workspace.package].
        let inherited = |k: &str| workspace_value(project, k);
        let version = s(pkg, "version")
            .or_else(|| inherited("version"))
            .unwrap_or_else(|| "0.1.0".into());
        let description = s(pkg, "description")
            .or_else(|| inherited("description"))
            .unwrap_or_default();
        let meta = pkg
            .get("metadata")
            .and_then(|m| m.get("peroxide"))
            .and_then(|p| p.as_table())
            .cloned()
            .unwrap_or_default();
        let bin = doc
            .get("bin")
            .and_then(|b| b.as_array())
            .and_then(|a| a.first())
            .and_then(|b| b.get("name"))
            .and_then(|n| n.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| name.clone());
        let exe = if cfg!(windows) {
            format!("{bin}.exe")
        } else {
            bin.clone()
        };
        let icon = s(&meta, "icon")
            .map(|i| project.join(i))
            .or_else(|| {
                // The usual places, if it isn't named.
                ["ui/images/app-icon.png", "icon.png", "assets/icon.png"]
                    .iter()
                    .map(|p| project.join(p))
                    .find(|p| p.is_file())
            })
            .filter(|p| p.is_file());
        Ok(Self {
            display_name: s(&meta, "display-name").unwrap_or_else(|| words(&name)),
            identifier: s(&meta, "identifier")
                .unwrap_or_else(|| format!("dev.perox.{}", name.replace('_', "-"))),
            publisher: s(&meta, "publisher").unwrap_or_default(),
            icon,
            binary: target_dir.join("release").join(exe),
            project: project.to_owned(),
            name,
            bin,
            version,
            description,
        })
    }

    /// `stock-dashboard-0.1.0`: the base of package file names.
    pub fn file_stem(&self) -> String {
        format!("{}-{}", self.name, self.version)
    }
}

#[cfg(test)]
pub(crate) fn sample(dir: &Path) -> AppInfo {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir.join("target/release")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"stock-dashboard\"\nversion = \"1.2.3\"\ndescription = \"Stocks at a glance\"\n\n[package.metadata.peroxide]\npublisher = \"Example Ltd\"\n",
    )
    .unwrap();
    let exe = if cfg!(windows) {
        "stock-dashboard.exe"
    } else {
        "stock-dashboard"
    };
    std::fs::write(
        dir.join("target/release").join(exe),
        b"\x7fELF pretend binary",
    )
    .unwrap();
    AppInfo::load(dir, &dir.join("target")).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_manifest() {
        let dir = std::env::temp_dir().join(format!("perox-pkg-info-{}", std::process::id()));
        let i = sample(&dir);
        assert_eq!(i.name, "stock-dashboard");
        assert_eq!(i.display_name, "Stock Dashboard");
        assert_eq!(i.version, "1.2.3");
        assert_eq!(i.identifier, "dev.perox.stock-dashboard");
        assert_eq!(i.publisher, "Example Ltd");
        assert_eq!(i.file_stem(), "stock-dashboard-1.2.3");
        assert!(i.binary.is_file());
        assert_eq!(i.icon, None);
        assert_eq!(words("my_cool-app"), "My Cool App");

        // A workspace member that inherits its version.
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join("app")).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\n[workspace.package]\nversion = \"4.5.6\"\n",
        )
        .unwrap();
        std::fs::write(
            ws.join("app/Cargo.toml"),
            "[package]\nname = \"app\"\nversion.workspace = true\n",
        )
        .unwrap();
        assert_eq!(
            AppInfo::load(&ws.join("app"), &ws.join("target"))
                .unwrap()
                .version,
            "4.5.6"
        );
    }
}
