# 0001. egui for the IDE and for user apps

- **Status:** Accepted
- **Date:** 2026-11-17

## Context
We need one toolkit that works on macOS, Windows, Linux and Android, that has a
permissive license, and that we can extend into a Qt-class RAD widget set. egui is
immediate mode. It has no retained widget tree, and its built-in layout is much weaker
than Qt's. The author has already shipped an egui app on Android (SolidCraft port).

## Decision
We will use **egui** (via eframe) for the IDE, and as the foundation of `perox-ui`, the
runtime that user apps use. RAD needs a retained, declarative layer, so we build it
ourselves on top of egui: `.pxui` forms ([0003](0003-form-format.md)) and layouts using
**taffy** (flexbox and grid). Out of the box egui looks the same on every platform rather
than native, so `perox-ui` ships theme packs (macOS-like, Fluent, Material).

## Consequences
- One toolkit to master. MIT/Apache licensing all the way down. A large crate ecosystem.
  AccessKit accessibility.
- We own the form language and the layout engine, which is real work.
- Android IME (soft keyboard) in egui is weak. We'll need to fix or contribute upstream.
- egui's API changes between releases (0.37 renamed `App::update` → `App::ui`, panels →
  `Panel`). We pin it in the Toolchain Set ([0005](0005-toolchain-sets.md)) and upgrade
  deliberately.

## Alternatives considered
- **Slint**: declarative, native styles, works on Android. But user apps would be bound
  by Slint's licenses (GPL / royalty-free with attribution / commercial), and we wouldn't
  own the form language.
- **iced**: Android support immature. **Makepad**: small ecosystem, unusual DSL.
  **Xilem**: too early. **gpui**: no Android, unstable API.
