# 0004. GPL-3.0 IDE, MIT/Apache runtime, CLA

- **Status:** Accepted (**get legal review before selling anything**)
- **Date:** 2026-11-17

## Context
The goal is open source that can also be sold. What users ship (their apps) must carry
no obligations. Qt's LGPL is a common reason people leave Qt. Our dependencies (rustc,
LLVM, egui) are MIT/Apache-2.0. Apache-2.0 is compatible with GPLv3, not with GPLv2.

## Decision
- **IDE: GPL-3.0-or-later.** Open source. Nobody can take it closed.
- **Everything that ends up in user apps: MIT OR Apache-2.0.** `perox-ui`, `perox-forms`,
  the component SDK, generated code. These crates must never depend on a GPL crate.
- **Revenue:**
  1. Sell the official signed and notarized builds. Source stays free to build yourself
     (the Ardour model).
  2. Commercial licenses for companies that want a non-GPL fork or to embed the IDE.
  3. Later: support and paid component packs.
- **CLA** ([CLA.md](../../CLA.md)) for every outside contribution, so (2) stays possible.
- **Trademark "PeroxIDE".** Forks can't ship under the name.

## Consequences
- Not distributable via the Mac App Store (GPL conflicts with its terms; the bundled
  toolchain can't be sandboxed anyway). Direct download is the plan.
- The CLA adds friction for contributors. It's needed.
- The CLA is a draft until a lawyer reviews it and `<LICENSOR>` is filled in.

## Alternatives considered
- MIT for everything: no revenue protection. AGPL: aimed at SaaS, not needed.
- FSL/BSL ("fair source"): not OSI open source, which conflicts with the goal.
