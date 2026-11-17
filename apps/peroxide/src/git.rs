//! Git, through the `git` command line: branch and file status (refreshed in the
//! background), the last-committed text of open files for the gutter, and commits.
//! If git isn't installed or the project isn't a repository, everything is simply off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use perox_editor::LineMark;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl FileState {
    pub fn letter(self) -> &'static str {
        match self {
            FileState::Modified => "M",
            FileState::Added => "A",
            FileState::Deleted => "D",
            FileState::Renamed => "R",
            FileState::Untracked => "U",
            FileState::Conflicted => "!",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    /// Absolute paths.
    pub files: Vec<(PathBuf, FileState)>,
}

#[derive(Debug, Default)]
pub struct Git {
    /// The repository's top folder, if the project is in one.
    pub root: Option<PathBuf>,
    pub status: Status,
    pending: Option<Receiver<Option<Status>>>,
    /// Last-committed text per file (`None`: not in the last commit).
    bases: HashMap<PathBuf, Option<String>>,
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

impl Git {
    /// Finds the repository containing `project`, if any.
    pub fn open(project: &Path) -> Self {
        let root = git(project, &["rev-parse", "--show-toplevel"]).map(|s| PathBuf::from(s.trim()));
        let root = root.map(|r| std::fs::canonicalize(&r).unwrap_or(r));
        Self {
            root,
            ..Default::default()
        }
    }

    pub fn state_of(&self, path: &Path) -> Option<FileState> {
        self.status
            .files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, s)| *s)
    }

    /// Re-reads status in the background (after saves, file changes, commits).
    pub fn refresh(&mut self, ctx: &egui::Context) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let status = git(&root, &["status", "--porcelain=v2", "--branch", "-z"])
                .map(|out| parse_status(&out, &root));
            let _ = tx.send(status);
            ctx.request_repaint();
        });
        self.pending = Some(rx);
        self.bases.clear(); // a commit or checkout may have changed them
    }

    /// Takes a finished refresh, if any. True if status changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(Some(status)) => {
                self.status = status;
                self.pending = None;
                true
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                false
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }

    /// The file's text in the last commit (cached).
    pub fn base(&mut self, path: &Path) -> Option<&str> {
        let root = self.root.clone()?;
        if !self.bases.contains_key(path) {
            let rel = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let text = git(&root, &["show", &format!("HEAD:{rel}")]);
            self.bases.insert(path.to_owned(), text);
        }
        self.bases.get(path)?.as_deref()
    }

    /// Commits: everything (`all`) or what's staged. Returns git's summary line.
    pub fn commit(&mut self, message: &str, all: bool) -> Result<String, String> {
        let root = self
            .root
            .clone()
            .ok_or("The project isn't in a git repository.")?;
        if message.trim().is_empty() {
            return Err("Write a commit message first.".into());
        }
        if all {
            run(&root, &["add", "-A"])?;
        }
        let out = run(&root, &["commit", "-m", message])?;
        Ok(out.lines().next().unwrap_or("Committed").to_owned())
    }

    pub fn diff(&self, path: Option<&Path>) -> Result<String, String> {
        let root = self
            .root
            .clone()
            .ok_or("The project isn't in a git repository.")?;
        let mut args = vec!["diff", "HEAD", "--"];
        let rel;
        if let Some(p) = path {
            rel = p
                .strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .into_owned();
            args.push(&rel);
        }
        run(&root, &args)
    }
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git isn't available: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &str, root: &Path) -> Status {
    let mut s = Status::default();
    let mut fields = out.split('\0').peekable();
    while let Some(entry) = fields.next() {
        if let Some(h) = entry.strip_prefix("# branch.head ") {
            s.branch = h.to_owned();
        } else if let Some(ab) = entry.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ');
            s.ahead = it
                .next()
                .and_then(|a| a.trim_start_matches('+').parse().ok())
                .unwrap_or(0);
            s.behind = it
                .next()
                .and_then(|b| b.trim_start_matches('-').parse().ok())
                .unwrap_or(0);
        } else if let Some(p) = entry.strip_prefix("? ") {
            s.files.push((join_git_path(root, p), FileState::Untracked));
        } else if entry.starts_with("1 ") || entry.starts_with("2 ") || entry.starts_with("u ") {
            let parts: Vec<&str> = entry
                .splitn(
                    if entry.starts_with('2') {
                        10
                    } else if entry.starts_with('u') {
                        11
                    } else {
                        9
                    },
                    ' ',
                )
                .collect();
            let xy = parts.get(1).copied().unwrap_or("..");
            let path = parts.last().copied().unwrap_or_default();
            let state = if entry.starts_with('u') {
                FileState::Conflicted
            } else if entry.starts_with('2') {
                fields.next(); // the original path of a rename
                FileState::Renamed
            } else if xy.contains('A') {
                FileState::Added
            } else if xy.contains('D') {
                FileState::Deleted
            } else {
                FileState::Modified
            };
            s.files.push((join_git_path(root, path), state));
        }
    }
    s.files.sort_by(|a, b| a.0.cmp(&b.0));
    s
}

/// Gutter marks: how `current` differs from `base`, by line of `current`.
pub fn line_marks(base: &str, current: &str) -> Vec<(usize, LineMark)> {
    use similar::{DiffOp, TextDiff};
    let diff = TextDiff::from_lines(base, current);
    let mut marks = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                marks.extend((new_index..new_index + new_len).map(|l| (l, LineMark::Added)));
            }
            DiffOp::Replace {
                new_index, new_len, ..
            } => {
                marks.extend((new_index..new_index + new_len).map(|l| (l, LineMark::Modified)));
            }
            DiffOp::Delete { new_index, .. } => marks.push((new_index, LineMark::DeletedAbove)),
            DiffOp::Equal { .. } => {}
        }
    }
    marks
}

/// Git prints `/`-separated paths; join them part by part so Windows paths use
/// backslashes throughout (Path equality ignores the mix, but display and hashing of
/// strings don't).
fn join_git_path(root: &Path, rel: &str) -> std::path::PathBuf {
    let mut out = root.to_path_buf();
    out.extend(rel.split('/').filter(|p| !p.is_empty()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.ab +2 -1\0\
                   1 .M N... 100644 100644 100644 h1 h2 src/main.rs\0\
                   1 A. N... 000000 100644 100644 h1 h2 src/new.rs\0\
                   2 R. N... 100644 100644 100644 h1 h2 R100 src/b.rs\0src/a.rs\0\
                   ? notes.txt\0";
        let s = parse_status(out, Path::new("/r"));
        assert_eq!((s.branch.as_str(), s.ahead, s.behind), ("main", 2, 1));
        let r = Path::new("/r");
        assert_eq!(
            s.files,
            [
                (r.join("notes.txt"), FileState::Untracked),
                (r.join("src").join("b.rs"), FileState::Renamed),
                (r.join("src").join("main.rs"), FileState::Modified),
                (r.join("src").join("new.rs"), FileState::Added),
            ]
        );
    }

    #[test]
    fn marks_added_modified_deleted_lines() {
        let base = "a\nb\nc\nd\n";
        let cur = "a\nB\nc\nnew\n";
        let marks = line_marks(base, cur);
        assert!(marks.contains(&(1, LineMark::Modified)), "{marks:?}");
        assert!(
            marks.contains(&(3, LineMark::Modified)) || marks.contains(&(3, LineMark::Added)),
            "{marks:?}"
        );
        assert_eq!(
            line_marks("a\nb\n", "a\n"),
            vec![(1, LineMark::DeletedAbove)]
        );
        assert_eq!(line_marks("a\n", "a\nb\n"), vec![(1, LineMark::Added)]);
    }

    #[test]
    fn real_repository_status_and_commit() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("perox-git-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.invalid"],
            vec!["config", "user.name", "T"],
        ] {
            run(&dir, &args).unwrap();
        }
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let mut g = Git::open(&dir);
        assert_eq!(g.root.as_deref(), Some(dir.as_path()));
        g.commit("first", true).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        g.refresh(&egui::Context::default());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !g.poll() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(g.status.branch, "main");
        assert_eq!(g.state_of(&dir.join("a.txt")), Some(FileState::Modified));
        assert_eq!(g.base(&dir.join("a.txt")), Some("one\n"));
        assert!(g.diff(None).unwrap().contains("+two"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
