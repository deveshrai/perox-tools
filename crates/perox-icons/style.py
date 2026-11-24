#!/usr/bin/env python3
"""Turns the Lucide sources in lucide/ into PeroxIDE's coloured icons in icons/.

The style: soft colour fills with a dark navy outline. Closed shapes are filled with the
icon's colour and outlined. Open lines that make up the icon on their own (arrows,
chevrons, a check mark) become thick coloured strokes with a navy edge; open lines drawn
over a filled shape (a document's text lines, the X on a circle) stay thin navy "ink".

Run from this folder after adding a Lucide SVG to lucide/:  python3 style.py
Give the new icon a colour (and any per-part overrides) in ICONS below.
"""
import os
import re
import sys

NAVY = "#26324d"
# On a dark theme navy disappears: outlines and detail lines are light instead, and
# coloured lines go without their dark edge.
DARK_INK = "#d3dcea"
OUTLINE = 1.6   # around filled shapes
EDGE = 3.8      # navy under a coloured line
LINE = 2.0      # the coloured line itself

G, R, Y, O = "#5cc87a", "#ec5f67", "#f7c548", "#f59e4c"
B, LB, T, P = "#5fa8ec", "#d4e6fb", "#3ebfae", "#9d7ce6"
PK, GR, LG, W = "#f28aa5", "#8a9bb4", "#e8edf4", "#ffffff"

# name: (colour, {element index: colour | "ink" | "line"}, lines)
#   colour  fills closed shapes and colours standalone lines
#   per-part overrides: a colour to fill that part, "ink" for a thin navy line,
#                       "line" for a thick coloured line, "fill" to fill an open part
#                       with the icon's colour (a box split by a line)
#   lines   "ink" or "line" for every open part (default: "ink" when the icon has a
#           filled shape, "line" otherwise)
ICONS = {
    "timer": (Y, {}, None),
    "menu": (B, {}, "line"),
    "square-menu": (LB, {}, None),
    "keyboard": (GR, {}, None),
    "mouse-pointer-click": (B, {}, None),
    "align-center-horizontal": (B, {1: "fill", 2: "fill", 3: "fill", 4: "fill"}, None),
    "align-center-vertical": (B, {1: "fill", 2: "fill", 3: "fill", 4: "fill"}, None),
    "align-end-horizontal": (B, {}, None),
    "align-end-vertical": (B, {}, None),
    "align-start-horizontal": (B, {}, None),
    "align-start-vertical": (B, {}, None),
    "align-horizontal-distribute-center": (T, {}, None),
    "align-vertical-distribute-center": (T, {}, None),
    "app-window": (LB, {}, None),
    "arrow-down-to-dot": (Y, {}, "line"),
    "arrow-up-from-dot": (B, {}, "line"),
    "arrow-down-to-line": (G, {}, "line"),
    "arrow-up-from-line": (B, {}, "line"),
    "arrow-down": (B, {}, None),
    "arrow-right": (B, {}, None),
    "arrow-up": (B, {}, None),
    "bug": (R, {}, None),
    "bug-play": (R, {0: G}, None),
    "case-sensitive": (B, {}, None),
    "chevron-down": (GR, {}, None),
    "chevron-right": (GR, {}, None),
    "circle-check": (G, {}, None),
    "circle-dashed": (GR, {}, None),
    "circle-dot": (R, {1: "#b33a4a"}, None),
    "circle-play": (G, {1: W}, None),
    "circle-slash": (LG, {}, None),
    "circle-x": (R, {}, None),
    "circle": (R, {}, None),
    "clipboard-paste": (Y, {}, None),
    "cloud-download": (B, {}, "line"),
    "cloud-upload": (B, {}, "line"),
    "code": (B, {}, None),
    "columns-3": (P, {}, None),
    "command": (GR, {}, None),
    "copy": (G, {1: "#a6e3b5"}, None),
    "eye": (B, {1: W}, None),
    "file": (LB, {}, None),
    "file-check": (LB, {}, None),
    "file-code": (LB, {}, None),
    "file-cog": (GR, {}, None),
    "file-diff": (LB, {}, None),
    "file-image": (LB, {}, None),
    "file-search": (B, {}, None),
    "file-text": (LB, {}, None),
    "flask-conical": (T, {}, None),
    "folder": (Y, {}, None),
    "folder-open": (Y, {}, None),
    "gauge": (O, {}, "line"),
    "git-branch": (G, {}, None),
    "git-commit-horizontal": (Y, {}, None),
    "git-compare": (O, {}, None),
    "git-merge": (R, {}, None),
    "grid-3x3": (LB, {}, None),
    "group": (B, {}, None),
    "ungroup": (B, {}, None),
    "hammer": (GR, {}, None),
    "hash": (GR, {}, None),
    "history": (B, {}, "line"),
    "image": (G, {1: Y}, None),
    "info": (B, {}, None),
    "layers": (P, {}, None),
    "layout-dashboard": (P, {}, None),
    "letter-text": (GR, {}, None),
    "lightbulb": (Y, {}, None),
    "list": (P, {}, None),
    "list-checks": (G, {}, None),
    "list-tree": (P, {}, None),
    "loader-circle": (B, {}, None),
    "log-out": (R, {}, "line"),
    "magnet": (R, {}, None),
    "message-square-warning": (Y, {}, None),
    "minus": (R, {}, None),
    "moon": (P, {}, None),
    "paintbrush": (O, {}, None),
    "panel-top": (LB, {}, None),
    "pause": (Y, {}, None),
    "pencil-ruler": (Y, {}, None),
    "play": (G, {}, None),
    "plug": (GR, {}, None),
    "plus": (G, {}, None),
    "puzzle": (P, {}, None),
    "rectangle-horizontal": (LB, {}, None),
    "redo-2": (Y, {}, None),
    "undo-2": (Y, {}, None),
    "redo-dot": (Y, {}, "line"),
    "refresh-cw": (T, {}, None),
    "regex": (P, {}, "line"),
    "replace": (B, {}, None),
    "replace-all": (B, {}, None),
    "rotate-ccw": (B, {}, None),
    "rows-3": (P, {}, None),
    "save": (B, {}, None),
    "save-all": (B, {}, None),
    "scissors": (GR, {}, "line"),
    "scroll-text": (Y, {}, None),
    "search-code": (B, {}, None),
    "separator-horizontal": (GR, {}, None),
    "settings": (GR, {1: LG}, None),
    "settings-2": (GR, {}, None),
    "shield-check": (G, {}, None),
    "sliders-horizontal": (B, {}, None),
    "space": (GR, {}, None),
    "square": (R, {}, None),
    "square-check": (G, {}, None),
    "square-chevron-down": (Y, {}, None),
    "square-dashed": (GR, {}, None),
    "square-mouse-pointer": (B, {}, None),
    "sun": (Y, {}, "line"),
    "table": (LB, {}, None),
    "chart-spline": (B, {0: "ink"}, "line"),
    "terminal": (T, {}, None),
    "text-cursor-input": (B, {}, None),
    "toggle-left": (G, {1: W}, None),
    "trash-2": (R, {}, None),
    "triangle-alert": (Y, {}, None),
    "type": (GR, {}, None),
    "variable": (P, {}, None),
    "whole-word": (B, {}, None),
    "x": (R, {}, None),
    "zap": (Y, {}, None),
}

ELEMENT = re.compile(r"<(path|rect|circle|ellipse|line|polyline|polygon)\b([^>]*?)/?>", re.S)


def is_closed(kind, attrs):
    if kind in ("rect", "circle", "ellipse", "polygon"):
        return True
    if kind == "path":
        d = re.search(r'\bd="([^"]*)"', attrs).group(1).strip()
        return d.endswith(("z", "Z"))
    return False


def style(name, src, dark=False):
    colour, parts, lines = ICONS[name]
    ink = DARK_INK if dark else NAVY
    # Detail lines drawn on a pale fill (a page, a window) need navy on any theme.
    detail = NAVY if colour in (LB, LG, W) else ink
    elements = [(m.group(1), m.group(2).strip()) for m in ELEMENT.finditer(src)]
    if not elements:
        raise SystemExit(f"{name}: no shapes found")
    has_fill = any(is_closed(k, a) for k, a in elements)
    default_lines = lines or ("ink" if has_fill else "line")
    fills, strokes = [], []
    for i, (kind, attrs) in enumerate(elements):
        part = parts.get(i)
        if part == "fill" or (is_closed(kind, attrs) and part not in ("ink", "line")):
            fill = colour if part in (None, "fill") else part
            fills.append(f'<{kind} {attrs} fill="{fill}" stroke="{ink}" stroke-width="{OUTLINE}"/>')
            continue
        how = part if part in ("ink", "line") else default_lines
        line_colour = part if part not in (None, "ink", "line") else colour
        if how == "ink":
            strokes.append(f'<{kind} {attrs} fill="none" stroke="{detail}" stroke-width="{OUTLINE}"/>')
        else:
            if not dark:
                strokes.append(f'<{kind} {attrs} fill="none" stroke="{NAVY}" stroke-width="{EDGE}"/>')
            strokes.append(f'<{kind} {attrs} fill="none" stroke="{line_colour}" stroke-width="{LINE}"/>')
    # Fills first, so detail lines are never hidden under a shape.
    body = "\n  ".join(fills + strokes)
    return (
        "<!-- Lucide 0.460.0 (ISC) shapes, PeroxIDE colours: see style.py -->\n"
        '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" '
        'stroke-linecap="round" stroke-linejoin="round">\n  ' + body + "\n</svg>\n"
    )


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    src_dir, out_dir = os.path.join(here, "lucide"), os.path.join(here, "icons")
    names = sorted(f[:-4] for f in os.listdir(src_dir) if f.endswith(".svg"))
    missing = [n for n in names if n not in ICONS]
    if missing:
        raise SystemExit("give these a colour in ICONS: " + ", ".join(missing))
    os.makedirs(os.path.join(out_dir, "dark"), exist_ok=True)
    for n in names:
        with open(os.path.join(src_dir, n + ".svg")) as f:
            src = f.read()
        for dark, folder in ((False, out_dir), (True, os.path.join(out_dir, "dark"))):
            with open(os.path.join(folder, n + ".svg"), "w") as f:
                f.write(style(n, src, dark))
    # A contact sheet, for looking at them all at once.
    if len(sys.argv) > 1:
        cols, cell = 12, 40
        rows = (len(names) + cols - 1) // cols
        for bg, file in (("#ffffff", "light"), ("#1e1e1e", "dark")):
            parts = []
            folder = os.path.join(out_dir, "dark") if file == "dark" else out_dir
            for i, n in enumerate(names):
                with open(os.path.join(folder, n + ".svg")) as f:
                    inner = re.search(r"<svg[^>]*>(.*)</svg>", f.read(), re.S).group(1)
                x, y = (i % cols) * cell + 8, (i // cols) * cell + 8
                parts.append(f'<g transform="translate({x},{y})" stroke-linecap="round" stroke-linejoin="round">{inner}</g>')
            sheet = (
                f'<svg xmlns="http://www.w3.org/2000/svg" width="{cols * cell + 16}" height="{rows * cell + 16}">'
                f'<rect width="100%" height="100%" fill="{bg}"/>' + "".join(parts) + "</svg>"
            )
            with open(os.path.join(sys.argv[1], f"sheet-{file}.svg"), "w") as f:
                f.write(sheet)
    print(f"styled {len(names)} icons")


if __name__ == "__main__":
    main()
