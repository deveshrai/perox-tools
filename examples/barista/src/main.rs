//! Barista: the touch panel of an espresso machine. Pick a drink, make it yours, press
//! Brew, and watch the cup fill. The window is `ui/main_window.slint`; this is its
//! code-behind. The cup, its layers, the stream and the steam are vector paths computed
//! here (`cup`), from the recipes (`drinks`).

mod cup;
mod drinks;

perox_ui::include_ui!();

use std::rc::Rc;
use std::time::{Duration, Instant};

use cup::Milk;
use drinks::{MENU, Options};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

/// Grinding before anything pours, and how long a full cup takes to pour.
const GRIND: f64 = 1.2;
const POUR: f64 = 6.0;
/// How long a fresh cup steams.
const STEAMING: f64 = 25.0;

struct Brew {
    started: Instant,
    recipe: Vec<(cup::Liquid, f64)>,
    drink: usize,
}

struct App {
    drink: usize,
    options: Options,
    brew: Option<Brew>,
    /// When the cup in the machine was finished (for the steam).
    done_at: Option<Instant>,
    born: Instant,
    water: f32,
    beans: f32,
    cups: i32,
}

impl Default for App {
    fn default() -> Self {
        Self {
            drink: 3,
            options: Options::default(),
            brew: None,
            done_at: None,
            born: Instant::now(),
            water: 0.82,
            beans: 0.64,
            cups: 12,
        }
    }
}

fn color(c: [u8; 3]) -> Color {
    Color::from_rgb_u8(c[0], c[1], c[2])
}

fn layer_model(layers: Vec<(String, [u8; 3])>) -> ModelRc<Layer> {
    let v: Vec<Layer> = layers
        .into_iter()
        .map(|(commands, c)| Layer {
            commands: commands.into(),
            color: color(c),
        })
        .collect();
    ModelRc::new(VecModel::from(v))
}

const STRENGTH: [&str; 5] = ["mild", "smooth", "balanced", "strong", "very strong"];

impl App {
    /// Everything that follows from the choices: the menu's cups, the summary, the
    /// price, and (unless brewing) the cup in the machine.
    fn refresh(&self, ui: &MainWindow) {
        let o = &self.options;
        let rows: Vec<ModelRc<DrinkData>> = MENU
            .chunks(4)
            .map(|row| {
                let tiles: Vec<DrinkData> = row
                    .iter()
                    .map(|d| DrinkData {
                        name: d.name.into(),
                        note: d.note.into(),
                        price: drinks::money(drinks::price(d, o)).into(),
                        layers: layer_model(drinks::layers(&drinks::recipe(d, o), 1.0, o, 0.0)),
                    })
                    .collect();
                ModelRc::new(VecModel::from(tiles))
            })
            .collect();
        ui.set_menu(ModelRc::new(VecModel::from(rows)));
        ui.set_cup(cup::cup_outline().into());
        ui.set_saucer(cup::saucer().into());
        let d = &MENU[self.drink];
        ui.set_selected_name(d.name.into());
        ui.set_summary(drinks::summary(d, o).into());
        let price = drinks::money(drinks::price(d, o));
        ui.set_price(price.clone().into());
        ui.set_strength(o.strength as i32);
        ui.set_strength_name(STRENGTH[(o.strength - 1) as usize].into());
        ui.set_size(o.size as i32);
        ui.set_sugar(o.sugar as i32);
        ui.set_water(self.water);
        ui.set_beans(self.beans);
        ui.set_cups_today(self.cups);
        if self.brew.is_none() {
            ui.set_brew_label(format!("Brew  ·  {price}").into());
            ui.set_cup_layers(layer_model(drinks::layers(
                &drinks::recipe(d, o),
                1.0,
                o,
                0.0,
            )));
            ui.set_steam_opacity(self.steam_strength(0.0) as f32);
        }
    }

    /// How much steam: hotter is more; a finished cup cools over `STEAMING` seconds.
    fn steam_strength(&self, since_done: f64) -> f64 {
        let heat = ((self.options.temperature as f64 - 55.0) / 35.0).clamp(0.2, 1.0);
        heat * (1.0 - since_done / STEAMING).clamp(0.0, 1.0)
    }

    fn read_options(&mut self, ui: &MainWindow) {
        self.options.milk = Milk::ALL[(ui.get_milk().clamp(0, 4)) as usize];
        self.options.temperature = ui.get_temperature().round() as i32;
        self.options.extra_shot = ui.get_extra_shot();
    }

    fn start(&mut self, ui: &MainWindow, already: f64) {
        if self.water < 0.08 {
            ui.set_status("Refill the water tank".into());
            ui.set_status_detail("The tank is nearly empty".into());
            return;
        }
        if self.beans < 0.06 {
            ui.set_status("Add beans".into());
            ui.set_status_detail("The hopper is nearly empty".into());
            return;
        }
        let d = &MENU[self.drink];
        self.brew = Some(Brew {
            started: Instant::now() - Duration::from_secs_f64(already),
            recipe: drinks::recipe(d, &self.options),
            drink: self.drink,
        });
        self.done_at = None;
        ui.set_brewing(true);
        ui.set_animating(true);
        ui.set_brew_label("Brewing…".into());
        ui.set_cup_layers(layer_model(Vec::new()));
        self.tick(ui);
    }

    /// The pouring time for this recipe: fuller cups take longer.
    fn pour_time(recipe: &[(cup::Liquid, f64)]) -> f64 {
        POUR * recipe.iter().map(|l| l.1).sum::<f64>().max(0.3)
    }
}

impl MainWindowHandlers for App {
    fn drink_picked(&mut self, ui: &MainWindow, name: SharedString) {
        if self.brew.is_some() {
            return;
        }
        if let Some(i) = MENU.iter().position(|d| d.name == name.as_str()) {
            self.drink = i;
            self.done_at = None;
            ui.set_status("Ready".into());
            ui.set_status_detail(format!("{}: {}", MENU[i].name, MENU[i].note).into());
            ui.set_screen("READY".into());
            ui.set_progress(0.0);
            self.refresh(ui);
        }
    }

    fn size_picked(&mut self, ui: &MainWindow, index: i32) {
        self.options.size = index.clamp(0, 2) as usize;
        self.refresh(ui);
    }

    fn strength_picked(&mut self, ui: &MainWindow, level: i32) {
        self.options.strength = level.clamp(1, 5) as u8;
        self.refresh(ui);
    }

    fn options_changed(&mut self, ui: &MainWindow) {
        self.read_options(ui);
        self.refresh(ui);
    }

    fn sugar_changed(&mut self, ui: &MainWindow, delta: i32) {
        self.options.sugar = (self.options.sugar as i32 + delta).clamp(0, 4) as u8;
        self.refresh(ui);
    }

    fn brew_clicked(&mut self, ui: &MainWindow) {
        if self.brew.is_none() {
            self.read_options(ui);
            self.start(ui, 0.0);
        }
    }

    fn tick(&mut self, ui: &MainWindow) {
        let now = self.born.elapsed().as_secs_f64();
        ui.set_steam(ModelRc::new(VecModel::from(
            cup::steam(now, 3)
                .into_iter()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )));
        let Some((started, recipe, drink)) = self
            .brew
            .as_ref()
            .map(|b| (b.started, b.recipe.clone(), b.drink))
        else {
            // A finished cup steams, then cools.
            let since = self.done_at.map_or(f64::MAX, |d| d.elapsed().as_secs_f64());
            let s = self.steam_strength(since);
            ui.set_steam_opacity(s as f32);
            if s <= 0.0 {
                ui.set_animating(false);
            }
            return;
        };
        let t = started.elapsed().as_secs_f64();
        let pour = Self::pour_time(&recipe);
        let total = GRIND + pour;
        ui.set_progress((t / total).min(1.0) as f32);
        let o = self.options;
        if t < GRIND {
            ui.set_status("Grinding beans".into());
            ui.set_status_detail(
                format!(
                    "{} grams, {}",
                    8 + 2 * o.strength as i32,
                    STRENGTH[(o.strength - 1) as usize]
                )
                .into(),
            );
            ui.set_screen("GRINDING".into());
            ui.set_pouring(false);
            return;
        }
        let p = ((t - GRIND) / pour).min(1.0);
        let phase = now * 2.0;
        ui.set_cup_layers(layer_model(drinks::layers(&recipe, p, &o, phase)));
        let filled: f64 = recipe.iter().map(|l| l.1).sum::<f64>() * p;
        ui.set_stream_end(cup::level_y(filled) as f32);
        ui.set_screen(format!("BREW {:>3}%", (p * 100.0).round()).into());
        match drinks::pouring(&recipe, p) {
            Some(l) if p < 1.0 => {
                ui.set_pouring(true);
                ui.set_stream_color(color(l.color(o.strength, o.milk)));
                ui.set_status(drinks::step_name(l).into());
                ui.set_status_detail(drinks::summary(&MENU[drink], &o).into());
            }
            _ => {
                // Done.
                let d = &MENU[drink];
                ui.set_pouring(false);
                ui.set_brewing(false);
                ui.set_status(format!("Enjoy your {}", d.name.to_lowercase()).into());
                ui.set_status_detail(drinks::summary(d, &o).into());
                ui.set_screen("ENJOY".into());
                self.water = (self.water - 0.03 * (o.size as f32 + 1.0)).max(0.0);
                self.beans = (self.beans
                    - 0.012 * (o.strength as f32 + if o.extra_shot { 2.0 } else { 0.0 }))
                .max(0.0);
                self.cups += 1;
                self.brew = None;
                self.done_at = Some(Instant::now());
                ui.set_cup_layers(layer_model(drinks::layers(
                    &drinks::recipe(d, &o),
                    1.0,
                    &o,
                    phase,
                )));
                self.refresh(ui);
                ui.set_brew_label("Brew another".into());
            }
        }
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }
}

/// Start-up options, for trying things out and for screenshots: `--drink <name>`,
/// `--milk oat|almond|skimmed|none`, `--size small|large`, `--strength 1-5`,
/// `--brew-at <seconds>` (start brewing, that far in).
fn apply_args(app: &mut App, ui: &MainWindow, args: &[String]) {
    let mut it = args.iter();
    let mut brew_at = None;
    while let Some(a) = it.next() {
        let v = it.clone().next().cloned().unwrap_or_default();
        match a.as_str() {
            "--drink" => {
                if let Some(i) = MENU.iter().position(|d| d.name.eq_ignore_ascii_case(&v)) {
                    app.drink = i;
                }
            }
            "--milk" => {
                if let Some(i) = Milk::ALL
                    .iter()
                    .position(|m| m.name().starts_with(&v.to_lowercase()))
                {
                    ui.set_milk(i as i32);
                }
            }
            "--size" => {
                app.options.size = drinks::SIZES
                    .iter()
                    .position(|s| s.eq_ignore_ascii_case(&v))
                    .unwrap_or(1)
            }
            "--strength" => app.options.strength = v.parse().unwrap_or(3u8).clamp(1, 5),
            "--brew-at" => brew_at = v.parse::<f64>().ok(),
            _ => continue,
        }
        it.next();
    }
    app.read_options(ui);
    app.refresh(ui);
    if let Some(t) = brew_at {
        app.start(ui, t);
    }
}

fn main() -> Result<(), slint::PlatformError> {
    // Slint's software renderer: its GPU renderers (Skia, FemtoVG) drew the glass and
    // saucer as boxes and dropped the steam (Slint 1.19). SLINT_BACKEND still overrides.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        slint::BackendSelector::new()
            .renderer_name("software".into())
            .select()?;
    }
    let ui = MainWindow::new()?;
    let app = ui.connect(App::default());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let weak = ui.as_weak();
    let first = Rc::clone(&app);
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        if let Some(ui) = weak.upgrade() {
            apply_args(&mut first.borrow_mut(), &ui, &args);
        }
    });
    ui.run()
}
