# 0011. Slint for user apps; `.slint` is the form format

- **Status:** Accepted
- **Date:** 2026-11-24
- **Supersedes:** the user-app half of [0001](0001-ui-toolkit.md), [0003](0003-form-format.md).
  **Amends:** [0004](0004-licensing.md), [0008](0008-preview-host.md).

## Context
After using the Phase 2 designer, egui-drawn forms looked and felt like egui, not like
desktop apps. Slint 1.19 (Nov 2026) has native-looking styles (Fluent, Cupertino,
Material, Qt), a mature declarative language with real layouts and absolute placement,
desktop features (drag and drop, tooltips, tray icons), Android and iOS. Its
`slint-interpreter` loads `.slint` at run time, renders to a pixel buffer with its
software renderer, and maps source offsets to on-screen rectangles (`internal-highlight`,
the API Slint's own live preview uses). Its compiler crate has a lossless syntax tree.
A spike confirmed all three inside our process.

## Decision
We will:
1. **Build user apps with Slint.** New GUI projects depend on `slint` (exact version
   pinned in the Toolchain Set) and `perox-ui`, our small runtime on top of it.
2. **Use `.slint` files as forms.** No private format. Anything Slint accepts can be
   written by hand; the designer edits the text in place through Slint's lossless parser
   (`perox-slint`), so comments and formatting survive.
3. **Keep RAD code-behind.** Event handlers are Slint callbacks on the root component
   (`callback ok_clicked();`, `clicked => { root.ok_clicked(); }`). The build generates a
   `MainWindowHandlers` trait and a `connect` method from them; "Go to handler" works as
   before.
4. **Render the designer canvas with `slint-interpreter`** and the software renderer into
   an egui texture, in process. The IDE itself stays egui.
5. **Ship message boxes and dialogs in `perox-ui`:** native message boxes through `rfd`
   (truly modal on every desktop OS), and `.slint` `Dialog` components shown with an
   `exec` helper. Slint 1.19 has no OS-level window modality; when it lands we adopt it
   behind the same API.

## Consequences
- **Licensing (amends 0004).** User apps link Slint, so each app must use one of Slint's
  licenses: GPL-3.0, the royalty-free license (desktop/mobile/web; requires the
  `AboutSlint` widget in an About dialog or the Slint badge on a public web page), or a
  paid commercial license. The "New GUI App" template ships an About dialog with
  `AboutSlint` so the royalty-free terms are met out of the box. Our own crates in user
  apps stay MIT OR Apache-2.0. The IDE links Slint under GPL-3.0; a commercially licensed
  IDE build would need Slint's royalty-free or commercial terms (desktop app with About
  dialog qualifies for royalty-free).
- **Preview host (amends 0008).** Third-party components written in `.slint` load
  directly in the interpreter: no preview host needed. Components with Rust logic still
  need the out-of-process host later.
- The designer depends on Slint internals (`internal-highlight`, `i-slint-compiler`
  syntax nodes), which have no stability guarantee. We pin `=1.19.0` and upgrade
  deliberately, with the designer tests as the gate.
- `perox-forms` and the egui form runtime are retired; `.pxui` projects aren't
  migrated (none exist outside this repo).
- User app builds are heavier (Slint's renderer and winit), roughly a minute on first
  build.

## Alternatives considered
- Keep egui for user apps: doesn't look native; the reason for this change.
- Keep `.pxui` and compile it to `.slint`: two languages to learn, and hand-written
  Slint couldn't be opened in the designer.
- Move the IDE to Slint too: no code editor, terminal or docking widgets; weeks of work
  for no user-visible gain. Revisit later.
