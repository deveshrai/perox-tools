# perox-ui

Draws `.pxui` forms in your app and in the PeroxIDE designer, using one code path, so the
preview is what ships. License: MIT OR Apache-2.0 (it ships inside your app).

```rust
perox_ui::run(include_str!("main_window.pxui"), |form, event| {
    if event.handler.as_deref() == Some("greet") {
        let name = form.widget("name").text();
        form.widget("status").set_text(format!("Hello, {name}!"));
    }
})
```

Usually you don't call this directly: the generated `MainWindow` wrapper and
`MainWindowHandlers` trait give typed access ([perox-forms](../perox-forms/README.md)).

## Layouts

| Element | Lays out children | Useful properties |
|---|---|---|
| `Window` | top to bottom | `title`, `size`, `min_size`, `resizable`, `margin` (10) |
| `VBox` / `HBox` | column / row (flexbox) | `spacing` (6), `margin`, `align`, `justify` |
| `Grid` | CSS grid, filled row by row | `columns: [0, 1]` (0 = fit, n = share of the rest) |
| `GroupBox` | top to bottom, framed | `title` |
| `Tabs` → `Tab` | one page at a time | `Tab { title }`, `Tabs { selected }` |

Every element accepts `width`, `height`, `min_width`, `max_width`, `min_height`,
`max_height`, `fixed_size: 200 x 30`, `stretch: n`, `visible`, `enabled`, `tooltip`, and
in a grid, `row`, `column`, `row_span`, `column_span`.

The defaults follow Qt's size policies:
- A window's, tab's or group box's only layout fills it.
- In rows and grids, fixed-height widgets sit centred.
- Along a row, text boxes, sliders, progress bars and lists grow.
- Tab sets, lists and multi-line text boxes take spare height.

## Widgets and events

| Widget | State property | Events |
|---|---|---|
| `Label` | `text` (also `heading`, `bold`, `color`, `font_size`, `wrap`, `text_align`) | — |
| `Button` | `text` | `clicked` |
| `CheckBox` | `checked` | `toggled` |
| `RadioButton` | `checked` (exclusive per parent or `group`) | `toggled` |
| `TextEdit` | `text` (`placeholder`, `rows`, `multiline`, `password`, `read_only`) | `changed`, `submitted` |
| `ComboBox` | `selected`, `items` | `changed` |
| `Slider` / `SpinBox` | `value` (`min`, `max`, `step`) | `changed` |
| `ProgressBar` | `value` (`min`, `max`, `text`) | — |
| `ListView` | `selected`, `items`, `rows` | `changed`, `activated` (double-click) |
| `Image` | `source`, `size` | — |
| `Separator`, `Spacer` | — | — |

State lives in properties: a value your code sets overrides the file
(`form.set(id, name, value)` or `form.widget(id).set_text(…)`).

`cargo run -p perox-ui --example gallery` shows everything.
