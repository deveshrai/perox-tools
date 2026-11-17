//! A PeroxIDE project is a Cargo package or workspace: its root, packages and targets
//! (from `cargo metadata`), and the file tree shown in the Explorer.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub name: String,
    /// `bin`, `lib`, `test`, `example`, `bench`, `proc-macro`…
    pub kind: String,
    pub src_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub name: String,
    pub manifest: PathBuf,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub packages: Vec<Package>,
}

/// A file or folder in the Explorer.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub children: Option<Vec<Node>>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.children.is_some()
    }
}

/// The Cargo.toml that owns `path`: the nearest one at or above it.
pub fn find_manifest(path: &Path) -> Option<PathBuf> {
    let start = if path.is_file() { path.parent()? } else { path };
    start
        .ancestors()
        .map(|d| d.join("Cargo.toml"))
        .find(|p| p.exists())
}

impl Project {
    /// Opens the project containing `path` (a folder or a file in it).
    pub fn open(path: &Path, cargo: &Path) -> Result<Self, String> {
        let manifest = find_manifest(path)
            .ok_or_else(|| format!("No Cargo.toml in {} or above it.", path.display()))?;
        let out = Command::new(cargo)
            .args([
                "metadata",
                "--no-deps",
                "--format-version",
                "1",
                "--manifest-path",
            ])
            .arg(&manifest)
            .output()
            .map_err(|e| format!("couldn't run cargo: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        Ok(Self::from_metadata(&meta))
    }

    pub fn from_metadata(meta: &Value) -> Self {
        let root = PathBuf::from(meta["workspace_root"].as_str().unwrap_or("."));
        let packages: Vec<Package> = meta["packages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Package {
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                manifest: PathBuf::from(p["manifest_path"].as_str().unwrap_or_default()),
                targets: p["targets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| Target {
                        name: t["name"].as_str().unwrap_or_default().to_owned(),
                        kind: t["kind"][0].as_str().unwrap_or("lib").to_owned(),
                        src_path: PathBuf::from(t["src_path"].as_str().unwrap_or_default()),
                    })
                    .collect(),
            })
            .collect();
        let name = if packages.len() == 1 {
            packages[0].name.clone()
        } else {
            root.file_name()
                .map_or("workspace".into(), |n| n.to_string_lossy().into_owned())
        };
        Self {
            root,
            name,
            packages,
        }
    }

    /// The binaries `cargo run` could start.
    pub fn binaries(&self) -> Vec<&Target> {
        self.packages
            .iter()
            .flat_map(|p| &p.targets)
            .filter(|t| t.kind == "bin")
            .collect()
    }

    /// The file tree, respecting .gitignore and skipping `target/` and hidden files.
    /// Folders first, then files, each alphabetical. At most `limit` entries.
    pub fn file_tree(&self, limit: usize) -> Node {
        let mut count = 0;
        build(&self.root, &mut count, limit, 0)
    }

    /// Every file in the tree, relative to the root (for quick open and agents).
    pub fn files(&self, limit: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in walker(&self.root).build().flatten() {
            if entry.file_type().is_some_and(|t| t.is_file()) {
                if let Ok(rel) = entry.path().strip_prefix(&self.root) {
                    out.push(rel.to_owned());
                }
                if out.len() >= limit {
                    break;
                }
            }
        }
        out.sort();
        out
    }
}

fn walker(root: &Path) -> ignore::WalkBuilder {
    let mut w = ignore::WalkBuilder::new(root);
    w.hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|e| e.file_name() != "target");
    w
}

fn build(dir: &Path, count: &mut usize, limit: usize, depth: usize) -> Node {
    let mut children = Vec::new();
    if depth < 32 {
        let mut w = walker(dir);
        w.max_depth(Some(1));
        let mut entries: Vec<_> = w.build().flatten().filter(|e| e.path() != dir).collect();
        entries.sort_by_key(|e| {
            (
                !e.file_type().is_some_and(|t| t.is_dir()),
                e.file_name().to_ascii_lowercase(),
            )
        });
        for e in entries {
            if *count >= limit {
                break;
            }
            *count += 1;
            if e.file_type().is_some_and(|t| t.is_dir()) {
                children.push(build(e.path(), count, limit, depth + 1));
            } else {
                children.push(Node {
                    name: e.file_name().to_string_lossy().into_owned(),
                    path: e.path().to_owned(),
                    children: None,
                });
            }
        }
    }
    Node {
        name: dir.file_name().map_or_else(
            || dir.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        path: dir.to_owned(),
        children: Some(children),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_a_real_project_and_lists_files() {
        let Some(cargo) = std::env::var_os("CARGO").map(PathBuf::from) else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("perox-project-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src/ui")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("src/ui/form.pxui"), "").unwrap();
        std::fs::write(dir.join("target/debug/junk"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();

        let p = Project::open(&dir.join("src/main.rs"), &cargo).unwrap();
        assert_eq!(p.name, "demo");
        assert_eq!(p.binaries().len(), 1);
        let tree = p.file_tree(1000);
        let names: Vec<_> = tree
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert_eq!(names, ["src", "Cargo.toml"]);
        let files = p.files(100);
        assert_eq!(
            files,
            [
                PathBuf::from("Cargo.toml"),
                PathBuf::from("src/main.rs"),
                PathBuf::from("src/ui/form.pxui")
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_manifest_is_a_clear_error() {
        let err = Project::open(Path::new("/"), Path::new("cargo")).unwrap_err();
        assert!(err.contains("No Cargo.toml"), "{err}");
    }
}
