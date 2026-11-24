# perox-icons

Every icon in the IDE, in one style: flat colour fills with outlines ("lineal colour").
The shapes are [Lucide](https://lucide.dev) 0.460.0 (ISC, `lucide/LICENSE-lucide`); the
colours are ours. No emoji anywhere in the UI: use an `Icon`.

- `lucide/`: the untouched Lucide sources.
- `style.py`: gives each icon its colour and writes `icons/` (navy outlines, for light
  themes) and `icons/dark/` (light outlines, for dark themes). Closed shapes are filled
  and outlined; lines that stand alone (arrows, chevrons) become thick coloured strokes;
  detail lines on a filled shape stay thin ink.
- `Icon::image` picks the variant for the current theme. Icons carry their own colours,
  so they aren't tinted; `Icon::faded` washes one out for "off".

To add an icon: put the Lucide SVG in `lucide/`, give it a colour in `style.py`'s `ICONS`,
run `python3 style.py`, and add a line to the `icons!` list in `src/lib.rs`.
