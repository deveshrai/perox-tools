//! Tests: finding them (`cargo test --no-run`, then each test binary's `--list`),
//! running some or all of them through cargo, and reading libtest's plain output back
//! into per-test results. Only stable libtest output is used (no `--format json`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

/// One compiled test harness: a library's, a binary's or an integration test's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binary {
    pub package: String,
    /// The target's name (`perox_build`, `gestures`, …).
    pub target: String,
    /// `lib`, `bin`, `test`, `example` or `bench`.
    pub kind: String,
    /// The target's root source file.
    pub src: PathBuf,
    /// The compiled harness.
    pub exe: PathBuf,
    /// Full test names (`module::tests::name`), as libtest lists them.
    pub tests: Vec<String>,
}

impl Binary {
    /// What identifies this harness in cargo's "Running" lines: the executable's name.
    pub fn key(&self) -> String {
        exe_key(&self.exe)
    }

    /// `cargo test` arguments selecting this target (and `test`, exactly, if given).
    pub fn cargo_args(&self, test: Option<&str>) -> Vec<String> {
        let mut args = vec!["-p".to_owned(), self.package.clone()];
        match self.kind.as_str() {
            "lib" | "proc-macro" | "rlib" | "dylib" | "cdylib" | "staticlib" => {
                args.push("--lib".into())
            }
            k => {
                args.push(format!("--{k}"));
                args.push(self.target.clone());
            }
        }
        if let Some(t) = test {
            args.extend(["--".into(), "--exact".into(), t.to_owned()]);
        }
        args
    }
}

fn exe_key(exe: &Path) -> String {
    exe.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What discovery found: the tests that built, and why the rest didn't.
#[derive(Debug, Clone, Default)]
pub struct Discovery {
    pub binaries: Vec<Binary>,
    /// The first compile error, if some tests don't build.
    pub error: Option<String>,
}

/// Builds the tests and lists them. Blocking: call it from a thread.
pub fn discover(cargo: &Path, root: &Path) -> Result<Discovery, String> {
    let out = Command::new(cargo)
        .args(["test", "--no-run", "--message-format=json"])
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .envs(crate::tools::env())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't run cargo: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut bins = binaries_from_artifacts(&stdout);
    let error = (!out.status.success()).then(|| {
        first_error(&stdout).unwrap_or_else(|| {
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .find(|l| l.starts_with("error"))
                .unwrap_or("The tests don't build.")
                .to_owned()
        })
    });
    for b in &mut bins {
        b.tests = list(&b.exe, b.src.parent().unwrap_or(root));
    }
    bins.sort_by(|a, b| {
        (&a.package, kind_order(&a.kind), &a.target).cmp(&(
            &b.package,
            kind_order(&b.kind),
            &b.target,
        ))
    });
    Ok(Discovery {
        binaries: bins,
        error,
    })
}

/// Asks a harness for its tests. A target with `harness = false` runs its own code
/// instead, so it gets ten seconds and an empty list if it doesn't answer like libtest.
fn list(exe: &Path, dir: &Path) -> Vec<String> {
    use std::io::Read;
    let Ok(mut child) = Command::new(exe)
        .args(["--list"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let mut stdout = child.stdout.take().expect("piped");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    // Something the program started may still hold the pipe open: don't wait on it.
    let text = rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap_or_default();
    parse_list(&text)
}

/// The first compiler error in cargo's JSON output, as `message (file:line)`.
fn first_error(json_lines: &str) -> Option<String> {
    json_lines.lines().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        let m = &v["message"];
        if v["reason"] != "compiler-message" || m["level"] != "error" {
            return None;
        }
        let text = m["message"].as_str()?;
        let span = m["spans"]
            .as_array()?
            .iter()
            .find(|s| s["is_primary"] == true);
        Some(match span {
            Some(s) => format!(
                "{text} ({}:{})",
                s["file_name"].as_str().unwrap_or_default(),
                s["line_start"]
            ),
            None => text.to_owned(),
        })
    })
}

fn kind_order(kind: &str) -> u8 {
    match kind {
        "lib" => 0,
        "bin" => 1,
        "test" => 2,
        _ => 3,
    }
}

/// Test harnesses among `cargo test --no-run --message-format=json`'s artifacts.
pub fn binaries_from_artifacts(json_lines: &str) -> Vec<Binary> {
    let mut out = Vec::new();
    for line in json_lines.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["reason"] != "compiler-artifact" || v["profile"]["test"] != true {
            continue;
        }
        let Some(exe) = v["executable"].as_str() else {
            continue;
        };
        let t = &v["target"];
        let kind = t["kind"][0].as_str().unwrap_or("lib").to_owned();
        out.push(Binary {
            package: package_name(v["package_id"].as_str().unwrap_or_default()),
            target: t["name"].as_str().unwrap_or_default().to_owned(),
            kind,
            src: PathBuf::from(t["src_path"].as_str().unwrap_or_default()),
            exe: PathBuf::from(exe),
            tests: Vec::new(),
        });
    }
    out
}

/// The name in a cargo package id: `path+file:///p/crates/a#0.1.0`,
/// `path+file:///p/crates/a#name@0.1.0` or the old `name 0.1.0 (path+…)`.
fn package_name(id: &str) -> String {
    if let Some((_, frag)) = id.rsplit_once('#') {
        if let Some((name, _)) = frag.split_once('@') {
            return name.to_owned();
        }
        let path = id.split('#').next().unwrap_or_default();
        return path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
    }
    id.split(' ').next().unwrap_or_default().to_owned()
}

/// Test names from a harness's `--list` output (`name: test` lines).
pub fn parse_list(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Ignored,
}

/// A finished test, as read from a run's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    /// The harness's executable name (see [`Binary::key`]); empty for doc tests.
    pub binary: String,
    pub name: String,
    pub outcome: Outcome,
}

/// Reads `cargo test` output line by line. Results come out as soon as libtest prints
/// them; a failure's captured output arrives later, in [`RunParser::output`].
#[derive(Debug, Default)]
pub struct RunParser {
    binary: String,
    /// Inside `---- name stdout ----`: the test and what's been read so far.
    capturing: Option<String>,
    /// Captured output of failed tests, by (binary, name).
    pub output: HashMap<(String, String), String>,
    /// libtest's summary lines (`test result: …`) for the run so far.
    pub summaries: Vec<String>,
}

impl RunParser {
    pub fn feed(&mut self, line: &str) -> Option<Finished> {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.trim_start().strip_prefix("Running ") {
            // `Running unittests src/lib.rs (target/debug/deps/a-123)`
            if let (Some(open), true) = (rest.rfind('('), rest.ends_with(')')) {
                self.binary = exe_key(Path::new(&rest[open + 1..rest.len() - 1]));
            }
            self.capturing = None;
            return None;
        }
        if trimmed.trim_start().starts_with("Doc-tests ") {
            self.binary.clear();
            self.capturing = None;
            return None;
        }
        if let Some(name) = trimmed
            .strip_prefix("---- ")
            .and_then(|r| r.strip_suffix(" stdout ----"))
        {
            self.capturing = Some(name.to_owned());
            self.output
                .insert((self.binary.clone(), name.to_owned()), String::new());
            return None;
        }
        if let Some(name) = &self.capturing {
            if trimmed == "failures:" || trimmed.starts_with("test result:") {
                self.capturing = None;
            } else {
                let key = (self.binary.clone(), name.clone());
                let text = self.output.entry(key).or_default();
                text.push_str(line);
                text.push('\n');
                return None;
            }
        }
        if trimmed.starts_with("test result:") {
            self.summaries.push(trimmed.to_owned());
            return None;
        }
        let rest = trimmed.strip_prefix("test ")?;
        let (name, result) = rest.rsplit_once(" ... ")?;
        let outcome = match result {
            "ok" => Outcome::Passed,
            "FAILED" => Outcome::Failed,
            r if r.starts_with("ignored") => Outcome::Ignored,
            _ => return None,
        };
        Some(Finished {
            binary: self.binary.clone(),
            name: name.to_owned(),
            outcome,
        })
    }
}

/// Where a test function is written, found from its module path: `a::b::tests::t` in a
/// target rooted at `src/lib.rs` is looked for in `src/a/b.rs` (or `src/a/b/mod.rs`,
/// or `src/a.rs` with `b` inline), then the function `t` in that file. Returns the
/// file and its 1-based line.
pub fn locate(src: &Path, name: &str) -> Option<(PathBuf, usize)> {
    let mut parts: Vec<&str> = name.split("::").collect();
    let func = parts.pop()?;
    let mut file = src.to_path_buf();
    // A crate root's modules sit next to it; any other file's in a folder named after it.
    let mut dir = src.parent().unwrap_or(Path::new("")).to_path_buf();
    for module in parts {
        let flat = dir.join(format!("{module}.rs"));
        let nested = dir.join(module).join("mod.rs");
        if flat.is_file() {
            file = flat;
            dir = dir.join(module);
        } else if nested.is_file() {
            file = nested;
            dir = dir.join(module);
        } else {
            // An inline module: still in this file; a file module inside it would be
            // in a subfolder.
            dir = dir.join(module);
        }
    }
    let text = std::fs::read_to_string(&file).ok()?;
    let needle = format!("fn {func}(");
    let line = text
        .lines()
        .position(|l| l.contains(&needle))
        .or_else(|| text.lines().position(|l| l.contains(&format!("fn {func}"))))?;
    Some((file, line + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_harnesses_from_artifacts() {
        let json = r#"{"reason":"compiler-artifact","package_id":"path+file:///w/crates/perox-build#0.3.0","target":{"kind":["lib"],"name":"perox_build","src_path":"/w/crates/perox-build/src/lib.rs"},"profile":{"test":true},"executable":"/w/target/debug/deps/perox_build-1a2b"}
{"reason":"compiler-artifact","package_id":"path+file:///w/x#xy@0.1.0","target":{"kind":["test"],"name":"gestures","src_path":"/w/x/tests/gestures.rs"},"profile":{"test":true},"executable":"/w/target/debug/deps/gestures-9f"}
{"reason":"compiler-artifact","package_id":"registry+https://x#serde@1.0.0","target":{"kind":["lib"],"name":"serde","src_path":"/r/lib.rs"},"profile":{"test":false},"executable":null}
{"reason":"build-finished","success":true}"#;
        let bins = binaries_from_artifacts(json);
        assert_eq!(bins.len(), 2);
        assert_eq!(
            (bins[0].package.as_str(), bins[0].kind.as_str()),
            ("perox-build", "lib")
        );
        assert_eq!(bins[0].key(), "perox_build-1a2b");
        assert_eq!(
            bins[0].cargo_args(Some("a::b")),
            ["-p", "perox-build", "--lib", "--", "--exact", "a::b"]
        );
        assert_eq!(bins[1].package, "xy");
        assert_eq!(bins[1].cargo_args(None), ["-p", "xy", "--test", "gestures"]);
        assert_eq!(package_name("foo 0.1.0 (path+file:///foo)"), "foo");
        let err = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","spans":[{"file_name":"tests/api.rs","line_start":2,"is_primary":true}]}}"#;
        assert_eq!(
            first_error(err).as_deref(),
            Some("mismatched types (tests/api.rs:2)")
        );
    }

    #[test]
    fn lists_tests() {
        let out = "a::one: test\nb::tests::two: test\nbench_x: benchmark\n\n2 tests, 1 benchmark\n";
        assert_eq!(parse_list(out), ["a::one", "b::tests::two"]);
    }

    #[test]
    fn parses_a_run() {
        let out = "\
   Compiling t v0.1.0 (/t)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.5s
     Running unittests src/lib.rs (target/debug/deps/t-abc)

running 3 tests
test tests::fine ... ok
test tests::later ... ignored, slow
test tests::broken ... FAILED

failures:

---- tests::broken stdout ----

thread 'tests::broken' panicked at src/lib.rs:9:9:
assertion `left == right` failed

failures:
    tests::broken

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/it.rs (target/debug/deps/it-def)

running 1 test
test works ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests t

running 1 test
test src/lib.rs - add (line 3) ... ok
";
        let mut p = RunParser::default();
        let got: Vec<Finished> = out.lines().filter_map(|l| p.feed(l)).collect();
        let short: Vec<(&str, &str, Outcome)> = got
            .iter()
            .map(|f| (f.binary.as_str(), f.name.as_str(), f.outcome))
            .collect();
        assert_eq!(
            short,
            [
                ("t-abc", "tests::fine", Outcome::Passed),
                ("t-abc", "tests::later", Outcome::Ignored),
                ("t-abc", "tests::broken", Outcome::Failed),
                ("it-def", "works", Outcome::Passed),
                ("", "src/lib.rs - add (line 3)", Outcome::Passed),
            ]
        );
        let output = &p.output[&("t-abc".to_owned(), "tests::broken".to_owned())];
        assert!(output.contains("panicked at src/lib.rs:9:9"), "{output}");
        assert!(!output.contains("failures:"));
        assert_eq!(p.summaries.len(), 2);
    }

    #[test]
    fn locates_test_functions() {
        let dir = std::env::temp_dir().join(format!("perox-locate-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("a")).unwrap();
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "mod a;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn top() {}\n}\n",
        )
        .unwrap();
        std::fs::write(src.join("a.rs"), "mod b;\n").unwrap();
        std::fs::write(
            src.join("a/b.rs"),
            "\n\n#[test]\nfn deep() {}\nmod tests {\n    #[test]\n    fn inner() {}\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("tests/it.rs"), "#[test]\nfn works() {}\n").unwrap();
        let lib = src.join("lib.rs");
        assert_eq!(locate(&lib, "tests::top"), Some((lib.clone(), 5)));
        assert_eq!(locate(&lib, "a::b::deep"), Some((src.join("a/b.rs"), 4)));
        assert_eq!(
            locate(&lib, "a::b::tests::inner"),
            Some((src.join("a/b.rs"), 7))
        );
        let it = dir.join("tests/it.rs");
        assert_eq!(locate(&it, "works"), Some((it.clone(), 2)));
        assert_eq!(locate(&lib, "nope"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_discovery_and_run() {
        let Some(cargo) = crate::tools::cargo() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!("perox-tests-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"tt\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn good() {}\n    #[test]\n    fn bad() { panic!(\"boom\") }\n}\n",
        )
        .unwrap();
        let found = discover(&cargo, &dir).expect("discovers");
        assert_eq!(found.error, None);
        let bins = found.binaries;
        let _ = std::fs::remove_dir_all(dir.join("target"));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(bins.len(), 1, "{bins:?}");
        assert_eq!(bins[0].tests, ["tests::bad", "tests::good"]);
        assert_eq!(bins[0].kind, "lib");
    }
}
