//! Every built-in widget in one form: `cargo run -p perox-ui --example gallery`.

const FORM: &str = r#"
Window gallery {
    title: "perox-ui gallery"
    size: 720 x 560

    VBox {
        Label heading { text: "Widget gallery" heading: true }
        Tabs tabs {
            Tab { title: "Inputs"
                Grid {
                    columns: [0, 1]
                    Label { text: "Name" }
                    TextEdit name { placeholder: "Your name" on changed: name_changed on submitted: greet }
                    Label { text: "Size" }
                    ComboBox size { items: ["Small", "Medium", "Large"] selected: 1 }
                    Label { text: "Volume" }
                    Slider volume { min: 0 max: 10 value: 3 on changed: volume_changed }
                    Label { text: "Copies" }
                    SpinBox copies { min: 1 max: 99 value: 1 }
                    Label { text: "Notes" }
                    TextEdit notes { rows: 3 }
                }
                GroupBox { title: "Options"
                    HBox {
                        CheckBox bold { text: "Bold" on toggled: bold_toggled }
                        RadioButton left { text: "Left" checked: true }
                        RadioButton right { text: "Right" }
                        Spacer {}
                    }
                }
            }
            Tab { title: "Lists"
                HBox {
                    ListView fruit { items: ["Apple", "Banana", "Cherry", "Damson", "Elderberry", "Fig"] on activated: fruit_activated }
                    Image { size: 96 x 96 }
                }
            }
        }
        Spacer {}
        Separator {}
        HBox {
            ProgressBar progress { value: 30 }
            Button greet { text: "Greet" on clicked: greet }
            Button quit { text: "Close" enabled: false }
        }
        Label status { text: "" color: #3a9a5b }
    }
}
"#;

fn main() -> eframe::Result {
    perox_ui::run(FORM, |form, event| match event.handler.as_deref() {
        Some("greet") => {
            let name = form.widget("name").text();
            let msg = if name.is_empty() {
                "Hello!".to_owned()
            } else {
                format!("Hello, {name}!")
            };
            form.widget("status").set_text(msg);
        }
        Some("volume_changed") => {
            let v = form.widget("volume").value();
            form.widget("progress").set_value(v * 10.0);
        }
        Some("fruit_activated") => {
            let f = form.widget("fruit").selected_text().unwrap_or_default();
            form.widget("status").set_text(format!("You picked {f}"));
        }
        _ => {}
    })
}
