//! Git, through the `git` command line: status (staged and unstaged), the last-committed
//! text of open files for the gutter, staging, discarding, commits, branches, history,
//! diffs, and fetch / pull / push in the background. The Source Control pane and the
//! `git.*` commands live here too. If git isn't installed or the project isn't a
//! repository, everything is simply off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};
use perox_core::{Access, CommandBus, CommandError, CommandSpec, Invocation};
use perox_editor::LineMark;
use perox_icons::Icon;
use serde_json::{Value, json};

use crate::commands::AppState;

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

    fn name(self) -> String {
        format!("{self:?}").to_lowercase()
    }

    /// From one side of porcelain v2's `XY`.
    fn from_code(c: char) -> Option<Self> {
        match c {
            'M' | 'T' => Some(FileState::Modified),
            'A' => Some(FileState::Added),
            'D' => Some(FileState::Deleted),
            'R' | 'C' => Some(FileState::Renamed),
            _ => None,
        }
    }
}

/// One changed file: what's staged, and what's changed in the working tree since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub staged: Option<FileState>,
    pub unstaged: Option<FileState>,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub branch: String,
    /// The upstream branch, if one is set.
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// Absolute paths, with their overall state (for the Explorer's colours).
    pub files: Vec<(PathBuf, FileState)>,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// "3 hours ago".
    pub when: String,
    pub subject: String,
}

/// What the Diff pane shows.
#[derive(Debug, Clone)]
pub struct DiffView {
    pub title: String,
    pub text: String,
}

/// Everything a background refresh reads.
#[derive(Debug, Default)]
struct Snapshot {
    status: Status,
    branches: Vec<Branch>,
    log: Vec<Commit>,
}

#[derive(Debug, Default)]
pub struct Git {
    /// The repository's top folder, if the project is in one.
    pub root: Option<PathBuf>,
    pub status: Status,
    pub branches: Vec<Branch>,
    /// Recent commits on the current branch, newest first.
    pub log: Vec<Commit>,
    pending: Option<Receiver<Option<Snapshot>>>,
    /// Last-committed text per file (`None`: not in the last commit).
    bases: HashMap<PathBuf, Option<String>>,
    /// A fetch, pull or push going on: its name and where its result arrives.
    remote: Option<(String, Receiver<Result<String, String>>)>,
    /// The last remote operation: name and result.
    pub remote_result: Option<(String, Result<String, String>)>,
    pub view: Option<DiffView>,
    /// Files the user asked to discard, waiting for confirmation.
    pub confirm_discard: Option<Vec<PathBuf>>,
    pub new_branch: Option<String>,
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

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        // Never wait for a password prompt nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("git isn't available: {e}"))?;
    if out.status.success() {
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        // fetch, pull and push report on stderr.
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(text)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

const NO_REPO: &str = "The project isn't in a git repository (or git isn't installed).";

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

    fn root(&self) -> Result<PathBuf, String> {
        self.root.clone().ok_or_else(|| NO_REPO.to_owned())
    }

    pub fn state_of(&self, path: &Path) -> Option<FileState> {
        self.status
            .files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, s)| *s)
    }

    /// Re-reads status, branches and history in the background (after saves, file
    /// changes, commits).
    pub fn refresh(&mut self, ctx: &egui::Context) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let snap =
                git(&root, &["status", "--porcelain=v2", "--branch", "-z"]).map(|out| Snapshot {
                    status: parse_status(&out, &root),
                    branches: git(
                        &root,
                        &[
                            "branch",
                            "--format=%(HEAD)%00%(refname:short)%00%(upstream:short)",
                        ],
                    )
                    .map(|o| parse_branches(&o))
                    .unwrap_or_default(),
                    log: git(
                        &root,
                        &["log", "-n", "50", &format!("--format={LOG_FORMAT}")],
                    )
                    .map(|o| parse_log(&o))
                    .unwrap_or_default(),
                });
            let _ = tx.send(snap);
            ctx.request_repaint();
        });
        self.pending = Some(rx);
        self.bases.clear(); // a commit or checkout may have changed them
    }

    /// Takes a finished refresh or remote operation, if any. True if status changed.
    pub fn poll(&mut self) -> bool {
        if let Some((name, rx)) = &self.remote
            && let Ok(result) = rx.try_recv()
        {
            self.remote_result = Some((name.clone(), result));
            self.remote = None;
        }
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(Some(snap)) => {
                self.status = snap.status;
                self.branches = snap.branches;
                self.log = snap.log;
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
            let rel = rel_git(&root, path);
            let text = git(&root, &["show", &format!("HEAD:{rel}")]);
            self.bases.insert(path.to_owned(), text);
        }
        self.bases.get(path)?.as_deref()
    }

    /// Commits what's staged; with `all`, stages every change first. Returns git's
    /// summary line.
    pub fn commit(&mut self, message: &str, all: bool) -> Result<String, String> {
        let root = self.root()?;
        if message.trim().is_empty() {
            return Err("Write a commit message first.".into());
        }
        if all {
            run(&root, &["add", "-A"])?;
        }
        let out = run(&root, &["commit", "-m", message])?;
        Ok(out.lines().next().unwrap_or("Committed").to_owned())
    }

    /// Unified diff: working tree against the index (`staged` false) or the index
    /// against the last commit (`staged` true). With no path, against the last commit.
    pub fn diff(&self, path: Option<&Path>, staged: bool) -> Result<String, String> {
        let root = self.root()?;
        let rel = path.map(|p| rel_git(&root, p));
        let mut args = vec!["diff"];
        match (&rel, staged) {
            (None, _) => args.push("HEAD"),
            (Some(_), true) => args.push("--cached"),
            (Some(_), false) => {}
        }
        args.push("--");
        if let Some(r) = &rel {
            args.push(r);
        }
        run(&root, &args)
    }

    /// The text to show for a changed file: its diff, or an untracked file's contents.
    pub fn diff_view(&self, path: &Path, staged: bool) -> Result<DiffView, String> {
        let root = self.root()?;
        let rel = rel_git(&root, path);
        let untracked = !staged
            && self
                .status
                .entries
                .iter()
                .any(|e| e.path == path && e.unstaged == Some(FileState::Untracked));
        let text = if untracked && path.is_dir() {
            // git lists a new folder as one entry.
            let mut files: Vec<String> = Vec::new();
            let mut stack = vec![path.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        if files.len() < 500 {
                            stack.push(p);
                        }
                    } else {
                        files.push(rel_git(&root, &p));
                    }
                }
            }
            files.sort();
            let more = if files.len() > 200 {
                format!("\n… and {} more", files.len() - 200)
            } else {
                String::new()
            };
            files.truncate(200);
            format!(
                "new folder: {rel}/\n{}{more}",
                files.iter().map(|f| format!("+ {f}\n")).collect::<String>()
            )
        } else if untracked {
            let body = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            format!(
                "new file: {rel}\n{}",
                body.lines().map(|l| format!("+{l}\n")).collect::<String>()
            )
        } else {
            self.diff(Some(path), staged)?
        };
        Ok(DiffView {
            title: format!("{rel} ({})", if staged { "staged" } else { "working tree" }),
            text,
        })
    }

    pub fn stage(&self, paths: &[PathBuf]) -> Result<(), String> {
        let root = self.root()?;
        let rels: Vec<String> = paths.iter().map(|p| rel_git(&root, p)).collect();
        let mut args = vec!["add", "-A", "--"];
        args.extend(rels.iter().map(String::as_str));
        run(&root, &args).map(|_| ())
    }

    pub fn unstage(&self, paths: &[PathBuf]) -> Result<(), String> {
        let root = self.root()?;
        let rels: Vec<String> = paths.iter().map(|p| rel_git(&root, p)).collect();
        let has_head = git(&root, &["rev-parse", "--verify", "-q", "HEAD"]).is_some();
        let mut args = if has_head {
            vec!["restore", "--staged", "--"]
        } else {
            // Before the first commit there's nothing to restore from.
            vec!["rm", "--cached", "-q", "-r", "--"]
        };
        args.extend(rels.iter().map(String::as_str));
        run(&root, &args).map(|_| ())
    }

    /// Throws away working-tree changes: tracked files go back to the index, untracked
    /// files are deleted.
    pub fn discard(&self, paths: &[PathBuf]) -> Result<(), String> {
        let root = self.root()?;
        let (untracked, tracked): (Vec<&PathBuf>, Vec<&PathBuf>) = paths.iter().partition(|p| {
            self.status
                .entries
                .iter()
                .any(|e| &e.path == *p && e.unstaged == Some(FileState::Untracked))
        });
        if !tracked.is_empty() {
            let rels: Vec<String> = tracked.iter().map(|p| rel_git(&root, p)).collect();
            let mut args = vec!["restore", "--worktree", "--"];
            args.extend(rels.iter().map(String::as_str));
            run(&root, &args)?;
        }
        for p in untracked {
            let gone = if p.is_dir() {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
            gone.map_err(|e| format!("couldn't delete {}: {e}", p.display()))?;
        }
        Ok(())
    }

    pub fn switch(&self, branch: &str, create: bool) -> Result<String, String> {
        let root = self.root()?;
        if branch.trim().is_empty() || branch.starts_with('-') {
            return Err("That isn't a branch name.".into());
        }
        let args = if create {
            vec!["switch", "-c", branch]
        } else {
            vec!["switch", branch]
        };
        run(&root, &args).map(|o| o.lines().next().unwrap_or_default().trim().to_owned())
    }

    pub fn history(&self, count: usize, path: Option<&Path>) -> Result<Vec<Commit>, String> {
        let root = self.root()?;
        let n = count.to_string();
        let format = format!("--format={LOG_FORMAT}");
        let rel = path.map(|p| rel_git(&root, p));
        let mut args = vec!["log", "-n", &n, &format];
        if let Some(r) = &rel {
            args.extend(["--", r]);
        }
        run(&root, &args).map(|o| parse_log(&o))
    }

    /// A commit's message and patch.
    pub fn show(&self, commit: &str) -> Result<DiffView, String> {
        let root = self.root()?;
        if commit.starts_with('-') {
            return Err("That isn't a commit.".into());
        }
        let text = run(
            &root,
            &["show", "--stat", "--patch", "--format=medium", commit],
        )?;
        Ok(DiffView {
            title: format!("commit {}", &commit[..commit.len().min(10)]),
            text,
        })
    }

    pub fn remote_busy(&self) -> Option<&str> {
        self.remote.as_ref().map(|(n, _)| n.as_str())
    }

    /// Starts `fetch`, `pull` or `push` in the background.
    pub fn start_remote(&mut self, op: &str, ctx: &egui::Context) -> Result<(), String> {
        let root = self.root()?;
        if self.remote.is_some() {
            return Err("A fetch, pull or push is already running.".into());
        }
        let args: Vec<String> = match op {
            "fetch" => vec!["fetch".into(), "--prune".into()],
            "pull" => vec!["pull".into(), "--ff-only".into()],
            // A new branch gets its upstream on the first push.
            "push" if self.status.upstream.is_none() => {
                vec!["push".into(), "-u".into(), "origin".into(), "HEAD".into()]
            }
            "push" => vec!["push".into()],
            _ => return Err(format!("unknown remote operation {op}")),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            let _ = tx.send(run(&root, &a));
            ctx.request_repaint();
        });
        self.remote = Some((op.to_owned(), rx));
        self.remote_result = None;
        Ok(())
    }
}

const LOG_FORMAT: &str = "%H%x00%h%x00%an%x00%ar%x00%s";

fn rel_git(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn parse_branches(out: &str) -> Vec<Branch> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\0');
            let head = f.next()?;
            let name = f.next()?.to_owned();
            let upstream = f.next().filter(|u| !u.is_empty()).map(str::to_owned);
            Some(Branch {
                name,
                current: head == "*",
                upstream,
            })
        })
        .collect()
}

pub fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.splitn(5, '\0').collect();
            let [hash, short, author, when, subject] = f[..] else {
                return None;
            };
            Some(Commit {
                hash: hash.into(),
                short: short.into(),
                author: author.into(),
                when: when.into(),
                subject: subject.into(),
            })
        })
        .collect()
}

/// Parses `git status --porcelain=v2 --branch -z`.
pub fn parse_status(out: &str, root: &Path) -> Status {
    let mut s = Status::default();
    let mut fields = out.split('\0').peekable();
    while let Some(entry) = fields.next() {
        if let Some(h) = entry.strip_prefix("# branch.head ") {
            s.branch = h.to_owned();
        } else if let Some(u) = entry.strip_prefix("# branch.upstream ") {
            s.upstream = Some(u.to_owned());
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
            let path = join_git_path(root, p);
            s.files.push((path.clone(), FileState::Untracked));
            s.entries.push(Entry {
                path,
                staged: None,
                unstaged: Some(FileState::Untracked),
            });
        } else if entry.starts_with("1 ") || entry.starts_with("2 ") || entry.starts_with("u ") {
            let n = if entry.starts_with('2') {
                10
            } else if entry.starts_with('u') {
                11
            } else {
                9
            };
            let parts: Vec<&str> = entry.splitn(n, ' ').collect();
            let xy: Vec<char> = parts.get(1).copied().unwrap_or("..").chars().collect();
            let path = join_git_path(root, parts.last().copied().unwrap_or_default());
            if entry.starts_with('2') {
                fields.next(); // the original path of a rename
            }
            let (staged, unstaged) = if entry.starts_with('u') {
                (None, Some(FileState::Conflicted))
            } else {
                (
                    xy.first().copied().and_then(FileState::from_code),
                    xy.get(1).copied().and_then(FileState::from_code),
                )
            };
            let overall = if entry.starts_with('u') {
                FileState::Conflicted
            } else if entry.starts_with('2') {
                FileState::Renamed
            } else if staged == Some(FileState::Added) {
                FileState::Added
            } else if staged == Some(FileState::Deleted) || unstaged == Some(FileState::Deleted) {
                FileState::Deleted
            } else {
                FileState::Modified
            };
            s.files.push((path.clone(), overall));
            s.entries.push(Entry {
                path,
                staged,
                unstaged,
            });
        }
    }
    s.files.sort_by(|a, b| a.0.cmp(&b.0));
    s.entries.sort_by(|a, b| a.path.cmp(&b.path));
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

// ── the Source Control pane ──

/// Colour for a file's git state (Explorer, Source Control).
pub fn color(ui: &egui::Ui, state: FileState) -> egui::Color32 {
    use FileState::*;
    let dark = ui.visuals().dark_mode;
    let c = |d: u32, l: u32| {
        let h = if dark { d } else { l };
        egui::Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    };
    match state {
        Modified | Renamed => c(0xe2c08d, 0x9a6700),
        Added | Untracked => c(0x73c991, 0x1a7f37),
        Deleted | Conflicted => c(0xf85149, 0xcf222e),
    }
}

fn small(ui: &mut egui::Ui, icon: Icon, tip: &str) -> bool {
    ui.add(egui::Button::image(icon.image(ui)).frame(false).small())
        .on_hover_text(tip)
        .clicked()
}

/// Draws Source Control. Returns commands to run.
pub fn show(ui: &mut egui::Ui, g: &mut Git, message: &mut String) -> Vec<(&'static str, Value)> {
    let mut out: Vec<(&'static str, Value)> = Vec::new();
    let Some(root) = g.root.clone() else {
        ui.weak(NO_REPO);
        return out;
    };
    // Branch, sync state and remote buttons.
    ui.horizontal(|ui| {
        ui.add(Icon::GitBranch.image(ui));
        let current = g.status.branch.clone();
        egui::ComboBox::from_id_salt("git_branch")
            .selected_text(RichText::new(&current).strong())
            .show_ui(ui, |ui| {
                for b in &g.branches {
                    if ui.selectable_label(b.current, &b.name).clicked() && !b.current {
                        out.push(("git.switch", json!({ "branch": b.name })));
                    }
                }
                ui.separator();
                if ui.button("New Branch…").clicked() {
                    g.new_branch = Some(String::new());
                }
            });
        let st = &g.status;
        if st.ahead + st.behind > 0 {
            ui.weak(format!("↑{} ↓{}", st.ahead, st.behind));
        } else if st.upstream.is_none() {
            ui.weak("not published");
        }
        let busy = g.remote_busy().is_some();
        for (icon, op, tip) in [
            (Icon::RefreshCw, "git.fetch", "Fetch"),
            (Icon::CloudDownload, "git.pull", "Pull (fast-forward only)"),
            (Icon::CloudUpload, "git.push", "Push"),
        ] {
            if ui
                .add_enabled(!busy, egui::Button::image(icon.image(ui)).small())
                .on_hover_text(tip)
                .clicked()
            {
                out.push((op, json!({})));
            }
        }
        if let Some(op) = g.remote_busy() {
            ui.spinner();
            ui.weak(format!("{op}…"));
        }
    });
    if let Some(name) = &mut g.new_branch {
        let mut done = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(name)
                    .hint_text("new-branch-name")
                    .desired_width(180.0),
            );
            r.request_focus();
            if ui.button("Create").clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                out.push((
                    "git.switch",
                    json!({ "branch": name.trim(), "create": true }),
                ));
                done = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
        if done {
            g.new_branch = None;
        }
    }
    if let Some((op, result)) = &g.remote_result {
        match result {
            Ok(_) => ui.weak(format!("{op} finished")),
            Err(e) => ui.colored_label(ui.visuals().error_fg_color, format!("{op} failed: {e}")),
        };
    }

    // Commit.
    ui.add(
        egui::TextEdit::multiline(message)
            .hint_text("Commit message")
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    let staged: Vec<&Entry> = g
        .status
        .entries
        .iter()
        .filter(|e| e.staged.is_some())
        .collect();
    let changed: Vec<&Entry> = g
        .status
        .entries
        .iter()
        .filter(|e| e.unstaged.is_some())
        .collect();
    ui.horizontal(|ui| {
        let has_message = !message.trim().is_empty();
        let (label, all) = if staged.is_empty() {
            ("Commit All", true)
        } else {
            ("Commit", false)
        };
        let tip = if all {
            "Nothing is staged: stages every change and commits"
        } else {
            "Commits the staged changes"
        };
        if ui
            .add_enabled(
                has_message && !g.status.entries.is_empty(),
                egui::Button::image_and_text(Icon::GitCommitHorizontal.image(ui), label),
            )
            .on_hover_text(tip)
            .clicked()
        {
            out.push((
                "git.commit",
                json!({ "message": message.clone(), "all": all }),
            ));
        }
    });

    // Discard confirmation.
    if let Some(paths) = g.confirm_discard.clone() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            let what = match paths.as_slice() {
                [one] => rel_git(&root, one),
                many => format!("{} files", many.len()),
            };
            ui.label(format!(
                "Discard the changes to {what}? This can't be undone."
            ));
            ui.horizontal(|ui| {
                let discard =
                    egui::Button::new(RichText::new("Discard").color(ui.visuals().error_fg_color));
                if ui.add(discard).clicked() {
                    out.push(("git.discard", json!({ "paths": paths, "confirm": true })));
                    g.confirm_discard = None;
                }
                if ui.button("Cancel").clicked() {
                    g.confirm_discard = None;
                }
            });
        });
    }
    ui.separator();

    let mut open_view: Option<(PathBuf, bool)> = None;
    egui::ScrollArea::vertical()
        .id_salt("git_scroll")
        .auto_shrink(false)
        .show(ui, |ui| {
            let rows = |ui: &mut egui::Ui,
                        list: &[&Entry],
                        staged_side: bool,
                        out: &mut Vec<(&'static str, Value)>,
                        open_view: &mut Option<(PathBuf, bool)>,
                        confirm: &mut Option<Vec<PathBuf>>| {
                for e in list {
                    let state = if staged_side { e.staged } else { e.unstaged }
                        .unwrap_or(FileState::Modified);
                    let shown = rel_git(&root, &e.path);
                    ui.horizontal(|ui| {
                        ui.add(crate::icons::for_file(&e.path).image(ui));
                        let r = ui.add(
                            egui::Label::new(RichText::new(&shown).color(color(ui, state)))
                                .sense(egui::Sense::click())
                                .truncate(),
                        );
                        if r.on_hover_text("Show the changes; double-click to open the file")
                            .clicked()
                        {
                            *open_view = Some((e.path.clone(), staged_side));
                        }
                        ui.colored_label(color(ui, state), state.letter());
                        if staged_side {
                            if small(ui, Icon::Minus, "Unstage") {
                                out.push(("git.unstage", json!({ "paths": [e.path] })));
                            }
                        } else {
                            if small(ui, Icon::Plus, "Stage") {
                                out.push(("git.stage", json!({ "paths": [e.path] })));
                            }
                            if small(ui, Icon::Undo2, "Discard changes") {
                                *confirm = Some(vec![e.path.clone()]);
                            }
                            if e.path.is_file() && small(ui, Icon::File, "Open the file") {
                                out.push(("file.open", json!({ "path": e.path })));
                            }
                        }
                    });
                }
            };
            if !staged.is_empty() {
                ui.horizontal(|ui| {
                    ui.strong(format!("Staged Changes ({})", staged.len()));
                    if small(ui, Icon::Minus, "Unstage all") {
                        out.push(("git.unstage", json!({ "all": true })));
                    }
                });
                rows(
                    ui,
                    &staged,
                    true,
                    &mut out,
                    &mut open_view,
                    &mut g.confirm_discard,
                );
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                ui.strong(format!("Changes ({})", changed.len()));
                if !changed.is_empty() {
                    if small(ui, Icon::Plus, "Stage all") {
                        out.push(("git.stage", json!({ "all": true })));
                    }
                    if small(ui, Icon::Undo2, "Discard all changes") {
                        g.confirm_discard = Some(changed.iter().map(|e| e.path.clone()).collect());
                    }
                }
            });
            if changed.is_empty() {
                ui.weak("No changes.");
            }
            rows(
                ui,
                &changed,
                false,
                &mut out,
                &mut open_view,
                &mut g.confirm_discard,
            );
            ui.add_space(8.0);
            egui::CollapsingHeader::new(RichText::new("History").strong())
                .default_open(true)
                .show(ui, |ui| {
                    if g.log.is_empty() {
                        ui.weak("No commits yet.");
                    }
                    for c in &g.log {
                        ui.horizontal(|ui| {
                            ui.monospace(RichText::new(&c.short).weak());
                            let r = ui.add(
                                egui::Label::new(&c.subject)
                                    .sense(egui::Sense::click())
                                    .truncate(),
                            );
                            if r.on_hover_text(format!("{} · {}", c.author, c.when))
                                .clicked()
                            {
                                out.push(("git.show", json!({ "commit": c.hash })));
                            }
                        });
                    }
                });
        });
    if let Some((path, staged)) = open_view {
        out.push((
            "git.diff",
            json!({ "path": path, "staged": staged, "show": true }),
        ));
    }
    out
}

/// Draws the Diff pane: a unified diff, coloured.
pub fn diff_pane(ui: &mut egui::Ui, g: &Git) {
    let Some(v) = &g.view else {
        ui.weak("Click a changed file or a commit in Source Control to see its changes here.");
        return;
    };
    ui.strong(&v.title);
    ui.separator();
    let dark = ui.visuals().dark_mode;
    let (add, del, hunk) = if dark {
        (
            egui::Color32::from_rgb(0x73, 0xc9, 0x91),
            egui::Color32::from_rgb(0xf8, 0x51, 0x49),
            egui::Color32::from_rgb(0x79, 0xc0, 0xff),
        )
    } else {
        (
            egui::Color32::from_rgb(0x1a, 0x7f, 0x37),
            egui::Color32::from_rgb(0xcf, 0x22, 0x2e),
            egui::Color32::from_rgb(0x09, 0x69, 0xda),
        )
    };
    egui::ScrollArea::both()
        .id_salt("diff_view")
        .auto_shrink(false)
        .show(ui, |ui| {
            if v.text.trim().is_empty() {
                ui.weak("No differences.");
            }
            for line in v.text.lines() {
                let t = RichText::new(line).monospace();
                let t = if line.starts_with("+++")
                    || line.starts_with("---")
                    || line.starts_with("diff ")
                    || line.starts_with("index ")
                {
                    t.weak()
                } else if line.starts_with('+') {
                    t.color(add)
                } else if line.starts_with('-') {
                    t.color(del)
                } else if line.starts_with("@@") {
                    t.color(hunk)
                } else {
                    t
                };
                ui.add(egui::Label::new(t).extend());
            }
        });
}

// ── commands ──

fn err(msg: impl Into<String>) -> CommandError {
    CommandError::Failed(msg.into())
}

fn rel(s: &AppState, p: &Path) -> String {
    s.project
        .as_ref()
        .and_then(|pr| p.strip_prefix(&pr.root).ok())
        .unwrap_or(p)
        .display()
        .to_string()
}

/// `paths` (relative to the project, or absolute) or every changed file with `all`.
fn paths_arg(
    s: &AppState,
    inv: &Invocation,
    pick: fn(&Entry) -> bool,
) -> Result<Vec<PathBuf>, CommandError> {
    if inv.bool_arg("all")?.unwrap_or(false) {
        return Ok(s
            .git
            .status
            .entries
            .iter()
            .filter(|e| pick(e))
            .map(|e| e.path.clone())
            .collect());
    }
    let list = inv
        .args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| err("Give `paths` (or `all: true`)."))?;
    let root = s
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| err("No project is open."))?;
    let mut out = Vec::new();
    for p in list {
        let p = p.as_str().ok_or_else(|| err("`paths` holds strings"))?;
        let path = root.join(p);
        let path = crate::ide::normalize_path(&path);
        if !path.starts_with(&root) {
            return Err(err("Paths must be inside the project."));
        }
        out.push(path);
    }
    Ok(out)
}

fn save_all(s: &mut AppState) -> Result<(), CommandError> {
    let dirty: Vec<crate::docs::DocId> = s
        .docs
        .list
        .iter()
        .filter(|d| d.is_dirty())
        .map(|d| d.id)
        .collect();
    for id in dirty {
        s.docs.save(id).map_err(err)?;
        s.lsp.did_save(id, &s.docs);
    }
    Ok(())
}

fn status_json(s: &AppState) -> Value {
    let st = &s.git.status;
    let side = |f: Option<FileState>| f.map(FileState::name);
    json!({
        "branch": st.branch, "upstream": st.upstream, "ahead": st.ahead, "behind": st.behind,
        "files": st.entries.iter().map(|e| json!({
            "path": rel(s, &e.path), "staged": side(e.staged), "unstaged": side(e.unstaged),
        })).collect::<Vec<_>>(),
        "remote": s.git.remote_busy(),
        "last_remote": s.git.remote_result.as_ref().map(|(op, r)| match r {
            Ok(out) => json!({ "operation": op, "ok": true, "output": out.trim() }),
            Err(e) => json!({ "operation": op, "ok": false, "error": e }),
        }),
    })
}

fn need_repo(s: &AppState) -> Result<(), CommandError> {
    if s.git.root.is_none() {
        return Err(err(NO_REPO));
    }
    Ok(())
}

fn remote(s: &mut AppState, op: &str) -> Result<Value, CommandError> {
    need_repo(s)?;
    let ctx = s.ctx.clone();
    s.git.start_remote(op, &ctx).map_err(err)?;
    s.layout_requests
        .push(crate::ide::LayoutRequest::ShowSourceControl);
    Ok(
        json!({ "started": op, "hint": "git.status reports `remote` while it runs and `last_remote` when done." }),
    )
}

pub fn register(bus: &mut CommandBus<AppState>) {
    type Handler = fn(&mut AppState, &Invocation) -> Result<Value, CommandError>;
    let paths = json!({ "type": "object", "properties": {
        "paths": { "type": "array", "items": { "type": "string" } }, "all": { "type": "boolean" }
    } });
    let commands: Vec<(CommandSpec, Handler)> = vec![
        (
            CommandSpec::new("git.status", "Git Status", Access::Read)
                .description("The branch, its upstream, commits ahead/behind, and each changed file's staged and unstaged state (modified, added, deleted, renamed, untracked, conflicted). Also any fetch/pull/push running and the last one's result."),
            |s, _| {
                need_repo(s)?;
                Ok(status_json(s))
            },
        ),
        (
            CommandSpec::new("git.diff", "Git Diff", Access::Read)
                .description("Unified diff. With `path`: the file's unstaged changes, or its staged ones with `staged: true`. Without: everything against the last commit. Only saved changes count. `show: true` also shows it in the Diff pane.")
                .params(json!({ "type": "object", "properties": { "path": { "type": "string" }, "staged": { "type": "boolean" }, "show": { "type": "boolean" } } })),
            |s, inv| {
                need_repo(s)?;
                let staged = inv.bool_arg("staged")?.unwrap_or(false);
                let path = match inv.args.get("path") {
                    Some(_) => Some(crate::ide::resolve(s, inv, "path")?),
                    None => None,
                };
                let text = match &path {
                    Some(p) => {
                        let v = s.git.diff_view(p, staged).map_err(err)?;
                        let text = v.text.clone();
                        if inv.bool_arg("show")?.unwrap_or(false) {
                            s.git.view = Some(v);
                            s.layout_requests.push(crate::ide::LayoutRequest::ShowPane(crate::layout::Pane::Diff));
                        }
                        text
                    }
                    None => s.git.diff(None, false).map_err(err)?,
                };
                Ok(json!(text))
            },
        ),
        (
            CommandSpec::new("git.stage", "Stage", Access::Execute)
                .description("Stages files (`paths`, relative to the project) or every change (`all: true`).")
                .params(paths.clone()),
            |s, inv| {
                need_repo(s)?;
                save_all(s)?;
                let p = paths_arg(s, inv, |e| e.unstaged.is_some())?;
                s.git.stage(&p).map_err(err)?;
                s.git.refresh(&s.ctx);
                Ok(json!({ "staged": p.len() }))
            },
        ),
        (
            CommandSpec::new("git.unstage", "Unstage", Access::Execute)
                .description("Takes files (`paths`) or everything (`all: true`) out of the staging area, keeping the changes.")
                .params(paths.clone()),
            |s, inv| {
                need_repo(s)?;
                let p = paths_arg(s, inv, |e| e.staged.is_some())?;
                s.git.unstage(&p).map_err(err)?;
                s.git.refresh(&s.ctx);
                Ok(json!({ "unstaged": p.len() }))
            },
        ),
        (
            CommandSpec::new("git.discard", "Discard Changes", Access::Execute)
                .description("Throws away working-tree changes to `paths` (or `all: true`): tracked files go back to their staged or committed text, untracked files are DELETED. Can't be undone, so it needs `confirm: true`.")
                .params(json!({ "type": "object", "required": ["confirm"], "properties": {
                    "paths": { "type": "array", "items": { "type": "string" } }, "all": { "type": "boolean" },
                    "confirm": { "type": "boolean" }
                } })),
            |s, inv| {
                need_repo(s)?;
                if inv.bool_arg("confirm")? != Some(true) {
                    return Err(err("Discarding can't be undone: pass `confirm: true` if that's really wanted."));
                }
                let p = paths_arg(s, inv, |e| e.unstaged.is_some())?;
                if p.iter().any(|p| s.docs.list.iter().any(|d| d.path == *p && d.is_dirty())) {
                    return Err(err("Some of those files have unsaved edits: save or close them first."));
                }
                s.git.discard(&p).map_err(err)?;
                s.git.refresh(&s.ctx);
                s.apply_disk_changes();
                Ok(json!({ "discarded": p.len() }))
            },
        ),
        (
            CommandSpec::new("git.commit", "Commit", Access::Execute)
                .description("Saves all files and commits the staged changes. With nothing staged, or `all: true`, stages every change first.")
                .params(json!({ "type": "object", "required": ["message"], "properties": { "message": { "type": "string" }, "all": { "type": "boolean" } } })),
            |s, inv| {
                need_repo(s)?;
                let message = inv.str_arg("message")?.to_owned();
                save_all(s)?;
                let nothing_staged = s.git.status.entries.iter().all(|e| e.staged.is_none());
                let all = inv.bool_arg("all")?.unwrap_or(nothing_staged);
                let summary = s.git.commit(&message, all).map_err(err)?;
                s.commit_message.clear();
                s.git.refresh(&s.ctx);
                s.status = summary.clone();
                Ok(json!({ "committed": summary }))
            },
        ),
        (
            CommandSpec::new("git.branches", "Branches", Access::Read)
                .description("Local branches, which one is checked out, and their upstreams."),
            |s, _| {
                need_repo(s)?;
                Ok(json!(s.git.branches.iter().map(|b| json!({ "name": b.name, "current": b.current, "upstream": b.upstream })).collect::<Vec<_>>()))
            },
        ),
        (
            CommandSpec::new("git.switch", "Switch Branch", Access::Execute)
                .description("Checks out a branch; `create: true` makes a new one from the current commit. Fails (changing nothing) if local changes would be overwritten.")
                .params(json!({ "type": "object", "required": ["branch"], "properties": { "branch": { "type": "string" }, "create": { "type": "boolean" } } })),
            |s, inv| {
                need_repo(s)?;
                let branch = inv.str_arg("branch")?.to_owned();
                save_all(s)?;
                let out = s.git.switch(&branch, inv.bool_arg("create")?.unwrap_or(false)).map_err(err)?;
                s.git.refresh(&s.ctx);
                s.apply_disk_changes();
                s.status = format!("On branch {branch}");
                Ok(json!({ "switched": branch, "git": out }))
            },
        ),
        (
            CommandSpec::new("git.log", "History", Access::Read)
                .description("Recent commits, newest first: hash, author, when, subject. `count` (default 20); `path` limits to one file's history.")
                .params(json!({ "type": "object", "properties": { "count": { "type": "integer", "default": 20 }, "path": { "type": "string" } } })),
            |s, inv| {
                need_repo(s)?;
                let n = inv.args.get("count").and_then(Value::as_u64).unwrap_or(20).min(500) as usize;
                let path = match inv.args.get("path") {
                    Some(_) => Some(crate::ide::resolve(s, inv, "path")?),
                    None => None,
                };
                let log = s.git.history(n, path.as_deref()).map_err(err)?;
                Ok(json!(log.iter().map(|c| json!({ "hash": c.hash, "short": c.short, "author": c.author, "when": c.when, "subject": c.subject })).collect::<Vec<_>>()))
            },
        ),
        (
            CommandSpec::new("git.show", "Show Commit", Access::Read)
                .description("A commit's message, changed files and patch, and shows it in the Diff pane.")
                .params(json!({ "type": "object", "required": ["commit"], "properties": { "commit": { "type": "string" } } })),
            |s, inv| {
                need_repo(s)?;
                let v = s.git.show(inv.str_arg("commit")?).map_err(err)?;
                let text = v.text.clone();
                s.git.view = Some(v);
                s.layout_requests.push(crate::ide::LayoutRequest::ShowPane(crate::layout::Pane::Diff));
                Ok(json!(text))
            },
        ),
        (
            CommandSpec::new("git.fetch", "Fetch", Access::Execute).description("Fetches from the remotes in the background."),
            |s, _| remote(s, "fetch"),
        ),
        (
            CommandSpec::new("git.pull", "Pull", Access::Execute).description("Pulls the upstream branch in the background (fast-forward only, so it never makes a merge commit)."),
            |s, _| remote(s, "pull"),
        ),
        (
            CommandSpec::new("git.push", "Push", Access::Execute).description("Pushes the current branch in the background; a branch without an upstream is pushed to origin and tracks it."),
            |s, _| remote(s, "push"),
        ),
    ];
    for (spec, h) in commands {
        bus.register(spec, h)
            .expect("git command ids are valid and unique");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let out = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
                   1 .M N... 100644 100644 100644 h1 h2 src/main.rs\0\
                   1 A. N... 000000 100644 100644 h1 h2 src/new.rs\0\
                   1 MM N... 100644 100644 100644 h1 h2 src/both.rs\0\
                   2 R. N... 100644 100644 100644 h1 h2 R100 src/b.rs\0src/a.rs\0\
                   ? notes.txt\0";
        let s = parse_status(out, Path::new("/r"));
        assert_eq!((s.branch.as_str(), s.ahead, s.behind), ("main", 2, 1));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        let r = Path::new("/r");
        assert_eq!(
            s.files,
            [
                (r.join("notes.txt"), FileState::Untracked),
                (r.join("src").join("b.rs"), FileState::Renamed),
                (r.join("src").join("both.rs"), FileState::Modified),
                (r.join("src").join("main.rs"), FileState::Modified),
                (r.join("src").join("new.rs"), FileState::Added),
            ]
        );
        let e = |p: &str| s.entries.iter().find(|e| e.path.ends_with(p)).unwrap();
        assert_eq!(
            (e("main.rs").staged, e("main.rs").unstaged),
            (None, Some(FileState::Modified))
        );
        assert_eq!(
            (e("new.rs").staged, e("new.rs").unstaged),
            (Some(FileState::Added), None)
        );
        assert_eq!(
            (e("both.rs").staged, e("both.rs").unstaged),
            (Some(FileState::Modified), Some(FileState::Modified))
        );
        assert_eq!(e("b.rs").staged, Some(FileState::Renamed));
    }

    #[test]
    fn parses_branches_and_log() {
        let b = parse_branches("*\0main\0origin/main\n \0feature\0\n");
        assert_eq!(b.len(), 2);
        assert!(b[0].current && b[0].upstream.as_deref() == Some("origin/main"));
        assert!(!b[1].current && b[1].upstream.is_none());
        let l = parse_log("abc123\0abc\0Ada\u{0}3 hours ago\0Fix: a \0 in subject?\n");
        assert_eq!(l[0].short, "abc");
        assert_eq!(l[0].subject, "Fix: a \0 in subject?");
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

    fn wait(g: &mut Git) {
        g.refresh(&egui::Context::default());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !g.poll() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn repo(name: &str) -> Option<PathBuf> {
        Command::new("git").arg("--version").output().ok()?;
        let dir = std::env::temp_dir().join(format!("perox-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.invalid"],
            vec!["config", "user.name", "T"],
            vec!["config", "commit.gpgsign", "false"],
        ] {
            run(&dir, &args).unwrap();
        }
        Some(dir)
    }

    #[test]
    fn real_repository_status_and_commit() {
        let Some(dir) = repo("basic") else { return };
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        let mut g = Git::open(&dir);
        assert_eq!(g.root.as_deref(), Some(dir.as_path()));
        g.commit("first", true).unwrap();
        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        wait(&mut g);
        assert_eq!(g.status.branch, "main");
        assert_eq!(g.state_of(&dir.join("a.txt")), Some(FileState::Modified));
        assert_eq!(g.base(&dir.join("a.txt")), Some("one\n"));
        assert!(g.diff(None, false).unwrap().contains("+two"));
        assert_eq!(g.log.len(), 1);
        assert_eq!(g.log[0].subject, "first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_staging_discard_and_branches() {
        let Some(dir) = repo("flow") else { return };
        let (a, b, n) = (dir.join("a.txt"), dir.join("b.txt"), dir.join("new.txt"));
        std::fs::write(&a, "a\n").unwrap();
        std::fs::write(&b, "b\n").unwrap();
        let mut g = Git::open(&dir);
        g.commit("first", true).unwrap();
        std::fs::write(&a, "a2\n").unwrap();
        std::fs::write(&b, "b2\n").unwrap();
        std::fs::write(&n, "new\n").unwrap();
        wait(&mut g);
        assert_eq!(g.status.entries.len(), 3);

        // Stage one file: only it is committed.
        g.stage(std::slice::from_ref(&a)).unwrap();
        wait(&mut g);
        let ea = g.status.entries.iter().find(|e| e.path == a).unwrap();
        assert_eq!((ea.staged, ea.unstaged), (Some(FileState::Modified), None));
        assert!(g.diff_view(&a, true).unwrap().text.contains("+a2"));
        g.unstage(std::slice::from_ref(&a)).unwrap();
        wait(&mut g);
        assert!(g.status.entries.iter().all(|e| e.staged.is_none()));
        g.stage(std::slice::from_ref(&a)).unwrap();
        g.commit("only a", false).unwrap();
        wait(&mut g);
        assert_eq!(g.status.entries.len(), 2, "{:?}", g.status.entries);

        // Discard: a tracked file goes back, an untracked one is deleted.
        assert!(g.diff_view(&n, false).unwrap().text.contains("+new"));
        g.discard(&[b.clone(), n.clone()]).unwrap();
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b\n");
        assert!(!n.exists());
        wait(&mut g);
        assert!(g.status.entries.is_empty());

        // Branches and history.
        g.switch("feature", true).unwrap();
        wait(&mut g);
        assert_eq!(g.status.branch, "feature");
        assert_eq!(g.branches.iter().filter(|b| b.current).count(), 1);
        assert_eq!(g.branches.len(), 2);
        g.switch("main", false).unwrap();
        let log = g.history(10, Some(&a)).unwrap();
        assert_eq!(
            log.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(),
            ["only a", "first"]
        );
        assert!(g.show(&log[0].hash).unwrap().text.contains("+a2"));
        assert!(g.switch("--force", false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_push_and_pull_through_a_local_remote() {
        let Some(dir) = repo("remote") else { return };
        let bare = dir.with_extension("bare");
        let _ = std::fs::remove_dir_all(&bare);
        run(
            &dir,
            &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
        )
        .unwrap();
        run(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        let mut g = Git::open(&dir);
        g.commit("first", true).unwrap();
        wait(&mut g);
        assert!(g.status.upstream.is_none());
        let ctx = egui::Context::default();
        let finish = |g: &mut Git| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while g.remote_busy().is_some() && std::time::Instant::now() < deadline {
                g.poll();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            g.remote_result.clone().unwrap().1
        };
        g.start_remote("push", &ctx).unwrap();
        finish(&mut g).expect("first push sets the upstream");
        wait(&mut g);
        assert_eq!(g.status.upstream.as_deref(), Some("origin/main"));
        g.start_remote("fetch", &ctx).unwrap();
        finish(&mut g).expect("fetch");
        g.start_remote("pull", &ctx).unwrap();
        finish(&mut g).expect("pull with nothing new");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bare);
    }
}
