# 0003. `.pxui` declarative forms + Rust code-behind

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
RAD users expect to draw a form, edit it by hand, and go back to the designer without
losing anything. Arbitrary egui code (loops, closures, conditionals) can't be opened in
a designer, because there's no stable widget tree to recover.

## Decision
We will describe forms in **`.pxui` files**, a small declarative language, with a
**lossless syntax tree** (rowan-style), so designer edits preserve comments, ordering and
formatting. The same grammar is also accepted inside Rust as `form! { … }`.

Each form has three parts:

| File | Owner | Contents |
|---|---|---|
| `main_window.pxui` | designer and people | widgets, layouts, properties, event names |
| `main_window.rs` | people | `impl MainWindowHandlers { fn on_save_clicked(…) }` |
| `main_window_ui.rs` (generated, in `OUT_DIR`) | build | plain egui code; never edited |

The designer **interprets** `.pxui` for instant preview. Release builds **compile** it to
plain egui code, so there's no runtime cost. Double-clicking an event creates or finds the
handler in the code-behind (syn edits, checked with rust-analyzer).

## Consequences
- True round-tripping between hand editing and the designer.
- We maintain a parser, formatter, interpreter and code generator, all in `perox-forms`
  (MIT/Apache, because it runs inside user builds).
- Hand-written egui code stays possible, but the designer won't open it.

## Alternatives considered
- RON/JSON: designer-friendly, unpleasant to hand-write. KDL: good, but bindings and
  expressions would be bolted on. Using the `.slint` language: ties us to Slint.
