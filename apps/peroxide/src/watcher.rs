//! Watches the project folder: files changed by other programs reload in their editors,
//! and the Explorer refreshes when files are added or removed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use notify::{EventKind, RecursiveMode, Watcher as _};

pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    changed: BTreeSet<PathBuf>,
    structure_changed: bool,
    /// Wait for a quiet moment before acting (editors and git write in bursts).
    quiet_since: Option<Instant>,
    ctx: egui::Context,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

/// What changed since the last call to [`Watcher::poll`].
#[derive(Debug, Default)]
pub struct Changes {
    /// Files whose contents changed.
    pub files: BTreeSet<PathBuf>,
    /// Files or folders were added, removed or renamed.
    pub structure: bool,
}

const SETTLE: Duration = Duration::from_millis(250);

fn ignored(path: &Path, root: &Path) -> bool {
    path.strip_prefix(root).ok().is_some_and(|rel| {
        rel.components()
            .next()
            .is_some_and(|c| c.as_os_str() == "target" || c.as_os_str() == ".git")
    })
}

impl Watcher {
    pub fn new(root: &Path, ctx: egui::Context) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let root_owned = root.to_owned();
        let wake = ctx.clone();
        let mut watcher = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            let relevant = ev
                .as_ref()
                .map_or(true, |e| e.paths.iter().any(|p| !ignored(p, &root_owned)));
            if relevant {
                let _ = tx.send(ev);
                wake.request_repaint();
            }
        })
        .ok()?;
        watcher.watch(root, RecursiveMode::Recursive).ok()?;
        Some(Self {
            _watcher: watcher,
            rx,
            changed: BTreeSet::new(),
            structure_changed: false,
            quiet_since: None,
            ctx,
        })
    }

    /// Collects events; returns them once things have been quiet for a moment.
    pub fn poll(&mut self, root: &Path) -> Option<Changes> {
        for ev in self.rx.try_iter().flatten() {
            self.quiet_since = Some(Instant::now());
            match ev.kind {
                EventKind::Create(_) | EventKind::Remove(_) => self.structure_changed = true,
                EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                    self.structure_changed = true
                }
                _ => {}
            }
            for p in ev.paths {
                if !ignored(&p, root) {
                    self.changed.insert(p);
                }
            }
        }
        let since = self.quiet_since?;
        if since.elapsed() < SETTLE {
            // Come back when it's been quiet long enough, even if nothing else happens.
            self.ctx.request_repaint_after(SETTLE - since.elapsed());
            return None;
        }
        self.quiet_since = None;
        Some(Changes {
            files: std::mem::take(&mut self.changed),
            structure: std::mem::take(&mut self.structure_changed),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_a_file_written_by_another_program() {
        let dir = std::env::temp_dir().join(format!("perox-watch-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let file = dir.join("src/a.rs");
        std::fs::write(&file, "one").unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();
        let mut w = Watcher::new(&root, egui::Context::default()).expect("watcher");
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(root.join("src/a.rs"), "two").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = None;
        while Instant::now() < deadline && seen.is_none() {
            std::thread::sleep(Duration::from_millis(100));
            seen = w.poll(&root);
        }
        let _ = std::fs::remove_dir_all(&dir);
        let changes = seen.expect("an event within 5 s");
        assert!(
            changes.files.contains(&root.join("src/a.rs")),
            "{:?}",
            changes.files
        );
    }

    #[test]
    fn ignores_build_output_and_git() {
        let root = Path::new("/p");
        assert!(ignored(Path::new("/p/target/debug/x"), root));
        assert!(ignored(Path::new("/p/.git/index"), root));
        assert!(!ignored(Path::new("/p/src/target.rs"), root));
    }
}
