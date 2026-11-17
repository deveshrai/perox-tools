//! A running cargo command: output lines, compiler diagnostics, and the result.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Check,
    Build,
    Run,
    Test,
    Clippy,
}

impl Kind {
    pub fn verb(self) -> &'static str {
        match self {
            Kind::Check => "check",
            Kind::Build => "build",
            Kind::Run => "run",
            Kind::Test => "test",
            Kind::Clippy => "clippy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Note,
}

/// A compiler message with its primary location.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub level: Level,
    pub message: String,
    pub file: Option<PathBuf>,
    /// 1-based, as rustc reports.
    pub line: usize,
    /// 1-based char column.
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
    /// rustc's full rendering, for the Output panel.
    pub rendered: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A line for the Output panel (cargo's progress, the program's output, test results).
    Line(String),
    Diagnostic(Diagnostic),
    Finished {
        success: bool,
    },
}

pub struct Job {
    pub kind: Kind,
    child: Child,
    events: Receiver<Event>,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job").field("kind", &self.kind).finish()
    }
}

impl Job {
    /// Starts `cargo <kind>` in `root`. `wake` runs whenever there's something new.
    pub fn start(
        cargo: &Path,
        root: &Path,
        kind: Kind,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let mut cmd = Command::new(cargo);
        cmd.arg(kind.verb())
            .arg("--message-format=json-diagnostic-rendered-ansi")
            .current_dir(root)
            .env("CARGO_TERM_COLOR", "never")
            .envs(crate::tools::env())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // so Stop also ends the program cargo started
        }
        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, rx) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let root = root.to_owned();
        {
            let (tx, wake) = (tx.clone(), wake.clone());
            thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    for e in parse_stdout_line(&line, &root) {
                        let _ = tx.send(e);
                    }
                    wake();
                }
            });
        }
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = tx.send(Event::Line(strip_ansi(&line)));
                wake();
            }
            wake();
        });
        Ok(Self {
            kind,
            child,
            events: rx,
        })
    }

    /// New events since the last call. Includes `Finished` once the process has exited
    /// and all output has been read.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out: Vec<Event> = self.events.try_iter().collect();
        if let Ok(Some(status)) = self.child.try_wait() {
            // Drain anything written just before exit.
            out.extend(self.events.try_iter());
            if self.events.try_recv().is_err() {
                out.push(Event::Finished {
                    success: status.success(),
                });
            }
        }
        out
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stops cargo and whatever it started.
    pub fn stop(&mut self) {
        let pid = self.child.id();
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .status();
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .status();
        }
        let _ = self.child.kill();
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.is_running() {
            self.stop();
        }
    }
}

/// cargo's stdout carries JSON messages, and with `run`/`test`, the program's own output.
pub fn parse_stdout_line(line: &str, root: &Path) -> Vec<Event> {
    if !line.starts_with('{') {
        return vec![Event::Line(line.to_owned())];
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return vec![Event::Line(line.to_owned())];
    };
    match v["reason"].as_str() {
        Some("compiler-message") => {
            let m = &v["message"];
            let level = match m["level"].as_str() {
                Some("error" | "error: internal compiler error") => Level::Error,
                Some("warning") => Level::Warning,
                _ => Level::Note,
            };
            let rendered = strip_ansi(m["rendered"].as_str().unwrap_or_default());
            let primary = m["spans"]
                .as_array()
                .and_then(|s| s.iter().find(|s| s["is_primary"] == true));
            let n = |v: &Value| v.as_u64().unwrap_or(1) as usize;
            let diag = Diagnostic {
                level,
                message: m["message"].as_str().unwrap_or_default().to_owned(),
                file: primary
                    .and_then(|p| p["file_name"].as_str())
                    .map(|f| root.join(f)),
                line: primary.map_or(1, |p| n(&p["line_start"])),
                column: primary.map_or(1, |p| n(&p["column_start"])),
                end_line: primary.map_or(1, |p| n(&p["line_end"])),
                end_column: primary.map_or(1, |p| n(&p["column_end"])),
                rendered: rendered.clone(),
            };
            let mut out = vec![Event::Diagnostic(diag)];
            out.extend(rendered.lines().map(|l| Event::Line(l.to_owned())));
            out
        }
        // Artifacts and build-script output aren't interesting in the Output panel.
        Some(_) => Vec::new(),
        None => vec![Event::Line(line.to_owned())],
    }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compiler_message() {
        let line = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","rendered":"\u001b[31merror\u001b[0m: mismatched types\n --> src/main.rs:2:18\n","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":18,"line_end":2,"column_end":20,"is_primary":true}]}}"#;
        let ev = parse_stdout_line(line, Path::new("/p"));
        let Event::Diagnostic(d) = &ev[0] else {
            panic!("{ev:?}")
        };
        assert_eq!(d.level, Level::Error);
        assert_eq!(d.file.as_deref(), Some(Path::new("/p/src/main.rs")));
        assert_eq!((d.line, d.column), (2, 18));
        assert!(d.rendered.starts_with("error: mismatched"));
        assert!(matches!(&ev[1], Event::Line(l) if l == "error: mismatched types"));
    }

    #[test]
    fn program_output_passes_through() {
        assert_eq!(
            parse_stdout_line("Hello, world!", Path::new("/")),
            vec![Event::Line("Hello, world!".into())]
        );
        assert!(parse_stdout_line(r#"{"reason":"compiler-artifact"}"#, Path::new("/")).is_empty());
    }

    #[test]
    fn real_cargo_build_reports_errors() {
        let Some(cargo) = crate::tools::cargo() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("perox-build-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"t\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    let x: u8 = \"no\";\n}\n",
        )
        .unwrap();
        let mut job = Job::start(&cargo, &dir, Kind::Check, || {}).unwrap();
        let mut events = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !events.iter().any(|e| matches!(e, Event::Finished { .. }))
            && std::time::Instant::now() < deadline
        {
            events.extend(job.poll());
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(events.contains(&Event::Finished { success: false }));
        let d = events
            .iter()
            .find_map(|e| match e {
                Event::Diagnostic(d) if d.level == Level::Error => Some(d),
                _ => None,
            })
            .expect("an error diagnostic");
        assert_eq!((d.line, d.column), (2, 17));
    }
}
