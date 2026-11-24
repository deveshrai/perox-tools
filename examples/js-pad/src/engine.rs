//! Running JavaScript: [Boa](https://boajs.dev), a JavaScript engine written in Rust,
//! with a `console` that writes to the app's console panel.

use std::cell::RefCell;

use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;
use boa_engine::{Context, JsResult, JsValue, NativeFunction, Source, js_string};

/// What kind of console line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Log,
    Info,
    Warn,
    Error,
    /// What you typed into the console.
    Input,
    /// What it evaluated to.
    Result,
    /// The app's own notes ("Ran in 3 ms").
    Note,
}

impl Level {
    pub fn color(self) -> [u8; 3] {
        match self {
            Level::Log => [0xd7, 0xda, 0xe0],
            Level::Info => [0x61, 0xaf, 0xef],
            Level::Warn => [0xe5, 0xc0, 0x7b],
            Level::Error => [0xff, 0x7b, 0x86],
            Level::Input => [0x8a, 0x93, 0xa3],
            Level::Result => [0x98, 0xc3, 0x79],
            Level::Note => [0x5c, 0x63, 0x70],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub level: Level,
    pub text: String,
}

thread_local! {
    /// Where `console.*` writes while a script runs (Boa's native functions can't
    /// capture Rust state that isn't traced, so this is per thread).
    static OUT: RefCell<Vec<Line>> = const { RefCell::new(Vec::new()) };
}

/// A value as the console shows it: strings as they are, everything else as JavaScript
/// would write it.
fn show(v: &JsValue, ctx: &mut Context) -> String {
    if let Some(s) = v.as_string() {
        return s.to_std_string_escaped();
    }
    // Arrays and plain objects: JSON (readable, one line); the rest: Boa's display.
    if v.is_object() && !v.is_callable() {
        let json = ctx
            .global_object()
            .get(js_string!("JSON"), ctx)
            .ok()
            .and_then(|j| j.as_object())
            .and_then(|j| j.get(js_string!("stringify"), ctx).ok())
            .and_then(|f| f.as_callable())
            .and_then(|f| {
                f.call(&JsValue::undefined(), std::slice::from_ref(v), ctx)
                    .ok()
            })
            .and_then(|s| s.as_string().map(|s| s.to_std_string_escaped()));
        if let Some(j) = json {
            return j;
        }
    }
    v.display().to_string()
}

fn write(level: Level, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = args
        .iter()
        .map(|a| show(a, ctx))
        .collect::<Vec<_>>()
        .join(" ");
    OUT.with(|o| o.borrow_mut().push(Line { level, text }));
    Ok(JsValue::undefined())
}

pub struct Engine {
    ctx: Context,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// A fresh global scope with `console`, and a limit so a runaway loop stops.
    pub fn new() -> Self {
        let mut ctx = Context::default();
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(20_000_000);
        ctx.runtime_limits_mut().set_recursion_limit(2_000);
        let console = ObjectInitializer::new(&mut ctx)
            .function(
                NativeFunction::from_fn_ptr(|_, a, c| write(Level::Log, a, c)),
                js_string!("log"),
                0,
            )
            .function(
                NativeFunction::from_fn_ptr(|_, a, c| write(Level::Info, a, c)),
                js_string!("info"),
                0,
            )
            .function(
                NativeFunction::from_fn_ptr(|_, a, c| write(Level::Warn, a, c)),
                js_string!("warn"),
                0,
            )
            .function(
                NativeFunction::from_fn_ptr(|_, a, c| write(Level::Error, a, c)),
                js_string!("error"),
                0,
            )
            .build();
        ctx.register_global_property(js_string!("console"), console, Attribute::all())
            .expect("a fresh context has no console");
        Self { ctx }
    }

    /// Runs a script; returns what it wrote, then any error.
    pub fn run(&mut self, src: &str) -> Vec<Line> {
        self.eval_inner(src, false)
    }

    /// Evaluates a line typed into the console: echoes it, then what it printed and its
    /// value (unless undefined).
    pub fn eval(&mut self, src: &str) -> Vec<Line> {
        let mut out = vec![Line {
            level: Level::Input,
            text: format!("› {src}"),
        }];
        out.extend(self.eval_inner(src, true));
        out
    }

    fn eval_inner(&mut self, src: &str, show_result: bool) -> Vec<Line> {
        OUT.with(|o| o.borrow_mut().clear());
        let result = self.ctx.eval(Source::from_bytes(src));
        // Promise callbacks (async functions, then()).
        let jobs = self.ctx.run_jobs();
        let mut out = OUT.with(|o| std::mem::take(&mut *o.borrow_mut()));
        match result {
            Ok(v) => {
                if show_result && !v.is_undefined() {
                    let text = show(&v, &mut self.ctx);
                    out.push(Line {
                        level: Level::Result,
                        text,
                    });
                }
            }
            Err(e) => out.push(Line {
                level: Level::Error,
                text: format!("Uncaught {e}"),
            }),
        }
        if let Err(e) = jobs {
            out.push(Line {
                level: Level::Error,
                text: format!("Uncaught (in promise) {e}"),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_and_results() {
        let mut e = Engine::new();
        let out = e.run("console.log('a', 1 + 1, [1, 2], {b: true}); console.warn('careful')");
        assert_eq!(
            out[0],
            Line {
                level: Level::Log,
                text: "a 2 [1,2] {\"b\":true}".into()
            }
        );
        assert_eq!(out[1].level, Level::Warn);
        // The console keeps the script's scope.
        e.run("const square = x => x * x;");
        let r = e.eval("[1, 2, 3].map(square)");
        assert_eq!(r[0].text, "› [1, 2, 3].map(square)");
        assert_eq!(
            r[1],
            Line {
                level: Level::Result,
                text: "[1,4,9]".into()
            }
        );
        assert_eq!(e.eval("let q = 1").len(), 1);
    }

    #[test]
    fn errors_and_promises() {
        let mut e = Engine::new();
        let out = e.run("null.x");
        assert_eq!(out[0].level, Level::Error);
        assert!(out[0].text.starts_with("Uncaught TypeError"), "{out:?}");
        let out =
            e.run("(async () => { await null; console.log('later'); })(); console.log('first')");
        let texts: Vec<_> = out.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["first", "later"]);
    }

    #[test]
    fn a_runaway_loop_stops() {
        let out = Engine::new().run("while (true) {}");
        assert_eq!(out.last().unwrap().level, Level::Error);
    }
}
