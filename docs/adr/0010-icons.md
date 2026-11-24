# 0010. Icons: Lucide SVGs, never emoji

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
Emoji render differently on every OS, look out of place in a professional tool, and can't be
tinted to the theme. Every toolbar button and menu item needs a proper icon.

## Decision
All UI icons are [Lucide](https://lucide.dev) SVGs, pinned at **0.460.0** (ISC license),
embedded through the `perox-icons` crate. They're stored white and tinted to the theme's text
colour (or a status colour) when drawn. **No emoji anywhere in the UI.** Commands map to icons in
`apps/peroxide/src/icons.rs`, so menus, toolbar and palette always agree.

## Consequences
- Consistent, crisp icons at any scale, in dark and light mode.
- Adding an icon means vendoring one more SVG from the same Lucide version.
- Typographic marks that aren't icons (the ● on an unsaved tab, ⇧⌘ in shortcut text) stay.

## Alternatives considered
- Codicons (VS Code's): CC-BY-4.0 needs attribution in the UI. Phosphor, Tabler: fine too;
  Lucide has the most IDE-relevant glyphs (regex, whole-word, case-sensitive, file kinds).

## Amendment (2026-11-24): colour style
Icons are now flat colour fills with outlines ("lineal colour"), still on Lucide's
shapes. `crates/perox-icons/style.py` generates a light-theme set (navy outlines) and a
dark-theme set (light outlines) from the untouched Lucide sources; the app picks one from
the theme and no longer tints icons.

