# perox-editor

The code editor. License: GPL-3.0-or-later.

| Module | What |
|---|---|
| `text` | Rope buffer, `Change`s, undo/redo (typing merges), `Edited` reports for tree-sitter and LSP |
| `selection` | Multiple cursors and selections, kept sorted and non-overlapping |
| `core` | Every editing operation: typing with auto-pairs, smart Enter, word movement, ⌘D, indent, comments, clipboard, completion insertion |
| `snippet` | LSP snippets and Tab-stop sessions |
| `syntax` | tree-sitter: incremental parse, highlight query over visible lines only |
| `completion` | Completion items, fuzzy scoring and ranking (prefix, word starts, recency) |
| `view` | The egui widget: draws visible lines, handles keys and mouse, completion popup, hover, signature help |
| `theme` | Colours, dark and light |

Positions are char indices; LSP columns are chars too (we negotiate `utf-32`). No UI code
outside `view` and `theme`, so the rest is unit-tested without a window.
