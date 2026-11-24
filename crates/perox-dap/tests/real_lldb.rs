//! Debugs a real program with the real adapter: breakpoints, the stack, locals, Rust
//! formatters, the console, stepping and exit. Skipped where no lldb-dap is installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use perox_dap::session::ConsoleKind;
use perox_dap::{Debugger, LaunchConfig, State, adapter};

const MAIN: &str = r#"fn total(items: &[u32]) -> u32 {
    let mut sum = 0;
    for i in items {
        sum += i;
    }
    sum
}

fn main() {
    let names = vec![String::from("ada"), String::from("grace")];
    let count = names.len();
    let sum = total(&[1, 2, 3]);
    println!("{count} names, sum {sum}");
}
"#;

fn wait(d: &mut Debugger, what: &str, done: impl Fn(&Debugger) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(d) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; state {:?}; console {:?}",
            d.session.state,
            d.session.console
        );
        d.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn build(dir: &Path) -> Option<PathBuf> {
    let cargo = perox_build::tools::cargo()?;
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"dbg_target\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rs"), MAIN).unwrap();
    let ok = Command::new(&cargo)
        .arg("build")
        .current_dir(dir)
        .envs(perox_build::tools::env())
        .status()
        .unwrap()
        .success();
    assert!(ok, "the target builds");
    let exe = dir.join("target/debug").join(if cfg!(windows) {
        "dbg_target.exe"
    } else {
        "dbg_target"
    });
    Some(exe)
}

fn sysroot() -> Option<PathBuf> {
    let cargo = perox_build::tools::cargo()?;
    let rustc = cargo.with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" });
    let out = Command::new(rustc)
        .args(["--print", "sysroot"])
        .envs(perox_build::tools::env())
        .output()
        .ok()?;
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

#[test]
fn debugs_a_real_program() {
    let Some(adapter) = adapter::find(None) else {
        eprintln!("no lldb-dap: skipped");
        return;
    };
    if cfg!(windows) {
        // LLDB's PDB support is best-effort (ADR 0012); this test pins down macOS and Linux.
        eprintln!("Windows: skipped");
        return;
    }
    let dir = std::env::temp_dir().join(format!("perox-dap-{}", std::process::id()));
    let Some(exe) = build(&dir) else { return };
    let dir = std::fs::canonicalize(&dir).unwrap();
    let main_rs = dir.join("src/main.rs");
    let init = sysroot()
        .map(|s| adapter::rust_init_commands(&s, None))
        .unwrap_or_default();
    let mut d = Debugger::start(
        &adapter,
        LaunchConfig {
            program: exe,
            cwd: dir.clone(),
            project_root: dir.clone(),
            init_commands: init,
            stop_on_panic: true,
            ..Default::default()
        },
        || {},
    )
    .expect("adapter starts");
    // Line 12: `let sum = total(&[1, 2, 3]);`, after `names` and `count` exist.
    d.session.set_breakpoints(&main_rs, &[12]);
    d.flush();
    wait(&mut d, "the breakpoint", |d| {
        d.session.is_stopped() && !d.session.variables.is_empty()
    });
    let bp = &d.session.breakpoints[&main_rs][0];
    assert!(bp.verified, "{bp:?}");
    let frame = d.session.current_frame().unwrap().clone();
    assert_eq!(frame.line, 12, "{frame:?}");
    assert!(frame.name.contains("main"), "{frame:?}");
    let locals = d.session.variables[&d.session.scopes[0].reference].clone();
    let count = locals
        .iter()
        .find(|v| v.name == "count")
        .expect("count is a local");
    assert_eq!(count.value, "2");
    let names = locals
        .iter()
        .find(|v| v.name == "names")
        .expect("names is a local");
    assert!(names.reference != 0, "a Vec has children: {names:?}");
    d.session.expand(names.reference);
    d.flush();
    let r = names.reference;
    wait(&mut d, "the Vec's elements", |d| {
        d.session.variables.contains_key(&r)
    });
    let elems = &d.session.variables[&r];
    assert!(
        elems.iter().any(|v| v.value.contains("ada")),
        "the Rust formatters show String contents: {elems:?}\nconsole: {:?}",
        d.session.console
    );

    d.session.evaluate("count");
    d.flush();
    wait(&mut d, "the console", |d| {
        d.session
            .console
            .iter()
            .any(|c| c.kind == ConsoleKind::Result)
    });

    // Step into `total`, then out again.
    d.session.step_in();
    d.flush();
    wait(&mut d, "step in", |d| {
        d.session.is_stopped() && d.session.current_frame().is_some()
    });
    assert!(
        d.session.current_frame().unwrap().name.contains("total"),
        "{:?}",
        d.session.frames
    );
    d.session.step_out();
    d.flush();
    wait(&mut d, "step out", |d| {
        d.session.is_stopped() && d.session.current_frame().is_some()
    });
    assert!(d.session.current_frame().unwrap().name.contains("main"));

    d.session.continue_();
    d.flush();
    wait(&mut d, "the end", |d| d.session.state == State::Terminated);
    let out: String = d.session.console.iter().map(|c| c.text.as_str()).collect();
    assert!(
        out.contains("2 names, sum 6"),
        "program output reaches the console: {out}"
    );
    assert_eq!(d.session.exit_code, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}
