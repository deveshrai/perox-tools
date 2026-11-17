//! The docking layout (egui_tiles): panes the user can drag, split and tab.

use eframe::egui::{self, RichText, WidgetText};
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tiles, Tree, UiResponse};

use crate::commands::Activity;
use crate::keymap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Explorer,
    Welcome,
    Activity,
    Problems,
}

/// Explorer on the left; editor area above, Activity and Problems below.
pub fn default_tree() -> Tree<Pane> {
    let mut tiles = Tiles::default();
    let explorer = tiles.insert_pane(Pane::Explorer);
    let welcome = tiles.insert_pane(Pane::Welcome);
    let activity = tiles.insert_pane(Pane::Activity);
    let problems = tiles.insert_pane(Pane::Problems);
    let left = tiles.insert_tab_tile(vec![explorer]);
    let center = tiles.insert_tab_tile(vec![welcome]);
    let bottom = tiles.insert_tab_tile(vec![activity, problems]);
    let right = tiles.insert_vertical_tile(vec![center, bottom]);
    let root = tiles.insert_horizontal_tile(vec![left, right]);
    set_shares(&mut tiles, right, &[(center, 3.0), (bottom, 1.0)]);
    set_shares(&mut tiles, root, &[(left, 1.0), (right, 4.0)]);
    Tree::new(egui::Id::unique("main_layout"), root, tiles)
}

fn set_shares(tiles: &mut Tiles<Pane>, container: TileId, shares: &[(TileId, f32)]) {
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(container) {
        for (id, share) in shares {
            linear.shares.set_share(*id, *share);
        }
    }
}

/// Draws the panes. Borrows the state it shows for one frame.
pub struct PaneUi<'a> {
    pub activity: &'a Activity,
}

impl egui_tiles::Behavior<Pane> for PaneUi<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> WidgetText {
        match pane {
            Pane::Explorer => "Explorer",
            Pane::Welcome => "Welcome",
            Pane::Activity => "Activity",
            Pane::Problems => "Problems",
        }
        .into()
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile: TileId, pane: &mut Pane) -> UiResponse {
        egui::Frame::new().inner_margin(8).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match pane {
                Pane::Explorer => {
                    ui.weak("No project open.");
                    ui.weak("Projects arrive in Phase 1.");
                }
                Pane::Welcome => welcome(ui),
                Pane::Activity => activity(ui, self.activity),
                Pane::Problems => {
                    ui.weak("No problems.");
                }
            }
        });
        UiResponse::None
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

fn welcome(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let shortcut = |id| {
        keymap::shortcut_for(id)
            .map(|s| ctx.format_shortcut(&s))
            .unwrap_or_default()
    };
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.2);
        ui.heading(RichText::new("PeroxIDE").size(32.0));
        ui.label("The Rust RAD IDE. Phase 0: the foundation.");
        ui.add_space(16.0);
        egui::Grid::new("welcome_keys")
            .num_columns(2)
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                ui.label("Command palette");
                ui.monospace(shortcut("view.command_palette"));
                ui.end_row();
                ui.label("Settings");
                ui.monospace(shortcut("app.settings"));
                ui.end_row();
                ui.label("Connect Claude Code");
                ui.label("Tools › MCP Server");
                ui.end_row();
            });
    });
}

fn activity(ui: &mut egui::Ui, log: &Activity) {
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink(false)
        .show(ui, |ui| {
            for e in log.iter() {
                ui.horizontal(|ui| {
                    ui.weak(&e.time);
                    if e.agent {
                        ui.colored_label(ui.visuals().warn_fg_color, "agent");
                    } else {
                        ui.weak("you");
                    }
                    ui.monospace(&e.text);
                });
            }
        });
}
