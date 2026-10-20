//! The ribbon (ds1-studio's `Ribbon.tsx`): a 34 px row with the app name,
//! the Home / View / Map / Help tabs, the Commands box, the file name and the
//! Sprites switch, over an 88 px body of labelled groups. Large buttons show
//! the icon above the label; consecutive small ones stack in columns of
//! three. Only commands whose feature exists are shown.
//!
//! Also the scripted "click every ribbon button" ([`ClickAll`]), which feeds
//! real pointer events to egui.

use std::collections::VecDeque;

use bevy_egui::egui::{
    self, Align2, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};
use d2_assets::GamePath;

use super::commands;
use super::theme::{self, icon};
use crate::config::AppMode;
use crate::doc::LayerKey;
use crate::state::AppState;

const TABS_HEIGHT: f32 = 34.0;
const BODY_HEIGHT: f32 = 88.0;
const LG: Vec2 = Vec2::new(58.0, 58.0);
const SM_HEIGHT: f32 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Size {
    Lg,
    Sm,
}

/// One thing in a ribbon group.
enum Item {
    /// A registry command with its ribbon label.
    Cmd(&'static str, Size, &'static str),
    /// A drop-down. `id` names it for scripts.
    Menu {
        id: &'static str,
        label: &'static str,
        glyph: char,
        size: Size,
        body: fn(&mut Ui, &mut AppState),
        /// Highlighted while this is true.
        active: fn(&AppState) -> bool,
    },
    Custom(fn(&mut Ui, &mut AppState)),
}

struct Group {
    label: &'static str,
    items: &'static [Item],
}

struct Tab {
    id: &'static str,
    label: &'static str,
    groups: &'static [Group],
}

fn never(_: &AppState) -> bool {
    false
}

fn capybara_select(state: &AppState) -> bool {
    use crate::edit::tools::Tool;
    state.edit.tool.selects() && state.edit.tool != Tool::SelectBox
}

const TABS: &[Tab] = &[
    Tab {
        id: "home",
        label: "Home",
        groups: &[
            Group {
                label: "File",
                items: &[
                    Item::Cmd("file.save", Size::Lg, "Save"),
                    Item::Menu {
                        id: "menu.recent",
                        label: "Recent",
                        glyph: icon::CLOCK,
                        size: Size::Sm,
                        body: recent_menu,
                        active: never,
                    },
                    Item::Menu {
                        id: "menu.profile",
                        label: "Profile",
                        glyph: icon::FOLDER,
                        size: Size::Sm,
                        body: profile_menu,
                        active: never,
                    },
                    Item::Menu {
                        id: "menu.import",
                        label: "Import",
                        glyph: icon::FILE_INPUT,
                        size: Size::Sm,
                        body: import_menu,
                        active: never,
                    },
                    Item::Menu {
                        id: "menu.export",
                        label: "Export",
                        glyph: icon::PACKAGE_CHECK,
                        size: Size::Sm,
                        body: export_menu,
                        active: never,
                    },
                    Item::Cmd("file.preferences", Size::Sm, "Preferences..."),
                    Item::Menu {
                        id: "menu.saves",
                        label: "Saves",
                        glyph: icon::PACKAGE_CHECK,
                        size: Size::Sm,
                        body: saves_menu,
                        active: never,
                    },
                    Item::Cmd("file.addInstall", Size::Sm, "Add game..."),
                ],
            },
            Group {
                label: "Edit",
                items: &[
                    Item::Cmd("edit.paste", Size::Lg, "Paste"),
                    Item::Cmd("edit.cut", Size::Sm, "Cut"),
                    Item::Cmd("edit.copy", Size::Sm, "Copy"),
                    Item::Cmd("edit.delete", Size::Sm, "Delete"),
                    Item::Cmd("edit.undo", Size::Sm, "Undo"),
                    Item::Cmd("edit.redo", Size::Sm, "Redo"),
                    Item::Menu {
                        id: "menu.flags",
                        label: "Flags",
                        glyph: icon::FLAG,
                        size: Size::Sm,
                        body: flags_menu,
                        active: never,
                    },
                    Item::Cmd("edit.moveToLayer", Size::Sm, "Move to layer..."),
                    Item::Cmd("edit.replace", Size::Sm, "Replace..."),
                ],
            },
            Group {
                label: "Tools",
                items: &[
                    Item::Cmd("tool.select", Size::Lg, "Select"),
                    Item::Menu {
                        id: "menu.select",
                        label: "Select",
                        glyph: icon::SQUARE_DASHED_POINTER,
                        size: Size::Lg,
                        body: select_menu,
                        active: capybara_select,
                    },
                    Item::Cmd("tool.paint", Size::Lg, "Paint"),
                    Item::Cmd("tool.erase", Size::Lg, "Erase"),
                    Item::Cmd("tool.pick", Size::Lg, "Pick"),
                ],
            },
            Group {
                label: "Paint / erase",
                items: &[
                    Item::Cmd("tool.freehand", Size::Sm, "Freehand"),
                    Item::Cmd("tool.rect", Size::Sm, "Rectangle"),
                    Item::Cmd("tool.fill", Size::Sm, "Fill area"),
                    Item::Custom(paint_group),
                ],
            },
            Group {
                label: "Layer",
                items: &[Item::Custom(active_layer)],
            },
        ],
    },
    Tab {
        id: "view",
        label: "View",
        groups: &[
            Group {
                label: "Navigate",
                items: &[
                    Item::Cmd("view.fit", Size::Lg, "Fit"),
                    Item::Cmd("view.zoom100", Size::Sm, "100%"),
                    Item::Cmd("view.zoomIn", Size::Sm, "Zoom in"),
                    Item::Cmd("view.zoomOut", Size::Sm, "Zoom out"),
                    Item::Cmd("view.focus", Size::Lg, "Just the map"),
                    Item::Cmd("view.game", Size::Sm, "Game view"),
                    Item::Menu {
                        id: "menu.gameSize",
                        label: "Game size",
                        glyph: super::studio::icons::FRAME,
                        size: Size::Sm,
                        body: game_size_menu,
                        active: never,
                    },
                ],
            },
            Group {
                label: "Mode",
                items: &[
                    Item::Cmd("mode.tiles", Size::Lg, "Tiles"),
                    Item::Cmd("mode.objects", Size::Lg, "Objects"),
                    Item::Cmd("mode.walk", Size::Sm, "Walkability"),
                    Item::Cmd("mode.automap", Size::Sm, "Automap"),
                    Item::Cmd("mode.light", Size::Sm, "Level light"),
                    Item::Cmd("mode.roofs", Size::Sm, "Roof hiding"),
                    Item::Cmd("view.popsInside", Size::Sm, "As if inside"),
                ],
            },
            Group {
                label: "Overlays",
                items: &[
                    Item::Cmd("view.rooms", Size::Sm, "Rooms 8x8"),
                    Item::Cmd("view.spawn", Size::Sm, "Spawn regions"),
                    Item::Cmd("view.warps", Size::Sm, "Warps"),
                    Item::Cmd("view.wallCategories", Size::Sm, "Wall categories"),
                ],
            },
            Group {
                label: "Show",
                items: &[
                    Item::Cmd("view.grid", Size::Sm, "Grid"),
                    Item::Cmd("view.minimap", Size::Sm, "Minimap"),
                    Item::Cmd("view.sprites", Size::Sm, "Sprites"),
                    Item::Cmd("view.animate", Size::Sm, "Animate"),
                    Item::Cmd("view.markers", Size::Sm, "Markers"),
                ],
            },
            Group {
                label: "Panels",
                items: &[
                    Item::Cmd("view.log", Size::Sm, "Log"),
                    Item::Cmd("view.objectsWindow", Size::Sm, "Objects"),
                    Item::Cmd("view.spritePreview", Size::Sm, "Sprite preview"),
                    Item::Cmd("view.history", Size::Sm, "History"),
                    Item::Cmd("view.foldLeft", Size::Sm, "Map list"),
                    Item::Cmd("view.foldRight", Size::Sm, "Tiles column"),
                ],
            },
            Group {
                label: "Look",
                items: &[Item::Menu {
                    id: "menu.theme",
                    label: "Theme",
                    glyph: icon::PALETTE,
                    size: Size::Lg,
                    body: theme_menu,
                    active: never,
                }],
            },
        ],
    },
    Tab {
        id: "map",
        label: "Map",
        groups: &[
            Group {
                label: "Map",
                items: &[
                    Item::Cmd("map.new", Size::Lg, "New map..."),
                    Item::Cmd("map.resize", Size::Lg, "Resize..."),
                    Item::Cmd("map.info", Size::Sm, "Map info"),
                    Item::Cmd("file.close", Size::Sm, "Close map"),
                ],
            },
            Group {
                label: "Tiles",
                items: &[
                    Item::Cmd("map.reroll", Size::Sm, "Re-roll floors..."),
                    Item::Cmd("map.reassign", Size::Sm, "Reassign tiles..."),
                    Item::Cmd("edit.replace", Size::Sm, "Find & replace..."),
                ],
            },
            Group {
                label: "Layers",
                items: &[
                    Item::Cmd("map.addFloor", Size::Sm, "Add floor layer"),
                    Item::Cmd("map.addWall", Size::Sm, "Add wall layer"),
                    Item::Cmd("map.addTag", Size::Sm, "Add tag layer"),
                ],
            },
            Group {
                label: "Tile libraries",
                items: &[
                    Item::Cmd("tiles.dt1Manager", Size::Lg, "DT1 libraries"),
                    Item::Cmd("tiles.settings", Size::Sm, "Tile settings"),
                    Item::Cmd("tiles.collision", Size::Sm, "Collision flags"),
                    Item::Cmd("tiles.paint", Size::Sm, "Paint pixels"),
                    Item::Cmd("tiles.actSafe", Size::Sm, "Make act-safe"),
                    Item::Cmd("tiles.review", Size::Sm, "Review libraries"),
                ],
            },
            Group {
                label: "Presets",
                items: &[
                    Item::Cmd("presets.save", Size::Sm, "Save selection..."),
                    Item::Cmd("presets.builder", Size::Sm, "Builder..."),
                    Item::Cmd("presets.suggest", Size::Sm, "Suggest..."),
                ],
            },
        ],
    },
    Tab {
        id: "game",
        label: "Game",
        groups: &[
            Group {
                label: "Levels",
                items: &[
                    Item::Cmd("game.addToGame", Size::Lg, "Add to game"),
                    Item::Cmd("game.changeType", Size::Lg, "Change level type"),
                    Item::Cmd("game.typePackages", Size::Lg, "Type packages"),
                ],
            },
            Group {
                label: "Text and items",
                items: &[
                    Item::Cmd("game.entryText", Size::Lg, "Entering text"),
                    Item::Cmd("game.cube", Size::Lg, "Cube recipe"),
                    Item::Cmd("game.customObject", Size::Lg, "Custom object"),
                ],
            },
            Group {
                label: "Data",
                items: &[
                    Item::Cmd("game.automap", Size::Lg, "Automap editor"),
                    Item::Cmd("game.tables", Size::Lg, "Tables"),
                ],
            },
        ],
    },
    Tab {
        id: "diag",
        label: "Diagnostics",
        groups: &[
            Group {
                label: "Check",
                items: &[
                    Item::Cmd("diag.compat", Size::Lg, "Compatibility"),
                    Item::Cmd("diag.crashLog", Size::Lg, "Crash log"),
                ],
            },
            Group {
                label: "Assets",
                items: &[
                    Item::Cmd("diag.unusedDt1", Size::Lg, "Unused DT1s"),
                    Item::Cmd("diag.restore", Size::Lg, "Restore assets"),
                ],
            },
        ],
    },
    Tab {
        id: "help",
        label: "Help",
        groups: &[Group {
            label: "Support",
            items: &[
                Item::Cmd("app.commands", Size::Lg, "Find a command"),
                Item::Cmd("help.shortcuts", Size::Lg, "Shortcuts"),
                Item::Cmd("help.about", Size::Lg, "About"),
            ],
        }],
    },
];

/// The tab ids, in order.
pub fn tab_ids() -> Vec<&'static str> {
    TABS.iter().map(|t| t.id).collect()
}

/// Every registry command the ribbon shows (tests check they exist).
#[cfg(test)]
pub fn ribbon_commands() -> Vec<&'static str> {
    let mut out = Vec::new();
    for t in TABS {
        for g in t.groups {
            for i in g.items {
                if let Item::Cmd(id, _, _) = i {
                    out.push(*id);
                }
            }
        }
    }
    out
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    let full = ui.available_rect_before_wrap();
    let tabs_rect = Rect::from_min_size(full.min, Vec2::new(full.width(), TABS_HEIGHT));
    ui.painter().rect_filled(tabs_rect, 0.0, s.bg2);
    ui.painter().line_segment(
        [tabs_rect.left_bottom(), tabs_rect.right_bottom()],
        Stroke::new(1.0, s.border),
    );
    let mut tabs_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(tabs_rect.shrink2(Vec2::new(10.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Max)),
    );
    tabs_row(&mut tabs_ui, state);
    ui.allocate_rect(tabs_rect, Sense::hover());

    let body_rect = Rect::from_min_size(
        Pos2::new(full.left(), tabs_rect.bottom()),
        Vec2::new(full.width(), BODY_HEIGHT),
    );
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    body.add_space(0.0);
    let tab = TABS
        .iter()
        .find(|t| t.id == state.config.ui.ribbon_tab)
        .unwrap_or(&TABS[0]);
    egui::ScrollArea::horizontal()
        .id_salt("ribbon_body")
        .auto_shrink([false, true])
        .show(&mut body, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(2.0, 1.0);
                let n = tab.groups.len();
                for (i, g) in tab.groups.iter().enumerate() {
                    group(ui, state, g, i + 1 == n);
                }
            });
        });
    ui.allocate_rect(body_rect, Sense::hover());
}

fn tabs_row(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    let fonts = state.shell.fonts_ready();
    // Brand.
    let (brand, _) = ui.allocate_exact_size(Vec2::new(176.0, TABS_HEIGHT), Sense::hover());
    let c = Pos2::new(brand.left() + 6.0, brand.center().y);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            c + Vec2::new(0.0, -4.5),
            c + Vec2::new(4.5, 0.0),
            c + Vec2::new(0.0, 4.5),
            c + Vec2::new(-4.5, 0.0),
        ],
        s.accent,
        Stroke::NONE,
    ));
    ui.painter().text(
        Pos2::new(brand.left() + 16.0, brand.center().y),
        Align2::LEFT_CENTER,
        "Capybara's D2 Editor",
        FontId::proportional(13.5),
        s.text,
    );
    // Tabs.
    for t in TABS {
        let on = state.config.ui.ribbon_tab == t.id
            || (t.id == "home" && !TABS.iter().any(|x| x.id == state.config.ui.ribbon_tab));
        let galley =
            ui.painter()
                .layout_no_wrap(t.label.to_owned(), FontId::proportional(13.0), s.text);
        let size = Vec2::new(galley.size().x + 24.0, TABS_HEIGHT - 4.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let id = tab_rect_id(t.id);
        state.shell.ribbon_rects.push((id, rect, true));
        if on {
            ui.painter().rect_filled(
                rect,
                CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                },
                s.panel,
            );
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.left(), rect.bottom() - 2.0),
                    rect.right_bottom(),
                ),
                0.0,
                s.accent,
            );
        }
        let color = if on {
            s.accent_text
        } else if resp.hovered() {
            s.text
        } else {
            s.muted
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);
        if resp.clicked() {
            state.shell.clicked.push(id);
            if !on {
                state.config.ui.ribbon_tab = t.id.to_owned();
                state.config_dirty = true;
            }
        }
    }
    // Right side: Sprites switch, file, Commands box (laid out right to left).
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let sprites = pill(ui, state, icon::IMAGE, "Sprites", "", false)
            .on_hover_text("Switch to Sprites mode (sprite viewer, editor and converter)");
        state
            .shell
            .ribbon_rects
            .push(("top.sprites", sprites.rect, true));
        if sprites.clicked() {
            state.config.mode = AppMode::Sprites;
            state.config_dirty = true;
        }
        if let Some(doc) = state.active_doc() {
            let path = doc.path.to_string().replace('\\', "/");
            let name = format!(
                "{}{}",
                doc.path.file_name(),
                if doc.is_dirty() { " *" } else { "" }
            );
            let avail = ui.available_width() - 180.0;
            ui.add(
                egui::Label::new(egui::RichText::new(path).size(12.0).color(s.muted))
                    .truncate()
                    .wrap_mode(egui::TextWrapMode::Truncate),
            )
            .on_hover_text(format!("{}\nfrom {}", doc.path, doc.source.layer_name));
            let _ = avail;
            ui.label(egui::RichText::new(name).strong().color(s.text));
        }
        let key = state.shell.key_of("app.commands");
        let resp = pill(ui, state, icon::SEARCH, "Commands", &key, true)
            .on_hover_text("Find a command by name");
        state
            .shell
            .ribbon_rects
            .push(("top.commands", resp.rect, true));
        if resp.clicked() {
            commands::run(state, "app.commands");
        }
    });
    let _ = fonts;
}

pub fn tab_rect_id(tab: &str) -> &'static str {
    match tab {
        "home" => "tab.home",
        "view" => "tab.view",
        "map" => "tab.map",
        "game" => "tab.game",
        "diag" => "tab.diag",
        _ => "tab.help",
    }
}

/// A rounded box in the top row (Commands, Sprites).
fn pill(
    ui: &mut Ui,
    state: &AppState,
    glyph: char,
    label: &str,
    key: &str,
    round: bool,
) -> egui::Response {
    let s = state.shell.shades;
    let font = FontId::proportional(12.0);
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), s.muted);
    let key_g = ui
        .painter()
        .layout_no_wrap(key.to_owned(), FontId::monospace(10.5), s.muted);
    let key_w = if key.is_empty() {
        0.0
    } else {
        key_g.size().x + 14.0
    };
    let size = Vec2::new(9.0 + 14.0 + 6.0 + text.size().x + key_w + 10.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fg = if resp.hovered() { s.text } else { s.muted };
    let radius = if round { 12 } else { 5 };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, if resp.hovered() { s.border2 } else { s.border }),
        StrokeKind::Inside,
    );
    ui.painter().text(
        Pos2::new(rect.left() + 9.0 + 7.0, rect.center().y),
        Align2::CENTER_CENTER,
        glyph,
        theme::icon_font(13.0, state.shell.fonts_ready()),
        fg,
    );
    let tx = rect.left() + 9.0 + 14.0 + 6.0;
    ui.painter().galley(
        Pos2::new(tx, rect.center().y - text.size().y / 2.0),
        text.clone(),
        fg,
    );
    if !key.is_empty() {
        let kr = Rect::from_min_size(
            Pos2::new(tx + text.size().x + 8.0, rect.center().y - 8.0),
            Vec2::new(key_g.size().x + 8.0, 16.0),
        );
        ui.painter().rect_stroke(
            kr,
            CornerRadius::same(3),
            Stroke::new(1.0, s.border),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(kr.center() - key_g.size() / 2.0, key_g, s.muted);
    }
    resp
}

fn group(ui: &mut Ui, state: &mut AppState, g: &Group, last: bool) {
    let s = state.shell.shades;
    let inner = ui.vertical(|ui| {
        ui.add_space(6.0);
        let items = ui
            .horizontal_top(|ui| {
                ui.set_min_height(64.0);
                let mut i = 0;
                while i < g.items.len() {
                    if is_small(&g.items[i]) {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let mut n = 0;
                            while i < g.items.len() && is_small(&g.items[i]) && n < 3 {
                                item(ui, state, &g.items[i]);
                                i += 1;
                                n += 1;
                            }
                        });
                    } else {
                        item(ui, state, &g.items[i]);
                        i += 1;
                    }
                }
            })
            .response
            .rect;
        let (label, _) = ui.allocate_exact_size(Vec2::new(items.width(), 14.0), Sense::hover());
        ui.painter().text(
            label.center(),
            Align2::CENTER_CENTER,
            g.label,
            FontId::proportional(10.5),
            s.muted,
        );
    });
    let r = inner.response.rect;
    ui.add_space(8.0);
    if !last {
        let x = ui.cursor().left();
        ui.painter().line_segment(
            [Pos2::new(x, r.top() + 4.0), Pos2::new(x, r.bottom() - 2.0)],
            Stroke::new(1.0, s.border),
        );
        ui.add_space(9.0);
    }
}

fn is_small(item: &Item) -> bool {
    match item {
        Item::Cmd(_, size, _) | Item::Menu { size, .. } => *size == Size::Sm,
        Item::Custom(_) => false,
    }
}

fn item(ui: &mut Ui, state: &mut AppState, item: &Item) {
    match item {
        Item::Cmd(id, size, label) => {
            let Some(cmd) = commands::find(id) else {
                return;
            };
            let enabled = commands::enabled(state, id);
            let on = commands::active(state, id);
            let key = state.shell.key_of(id);
            let tip = if key.is_empty() {
                cmd.label.to_owned()
            } else {
                format!("{} ({key})", cmd.label)
            };
            let resp = button(ui, state, label, cmd.icon, *size, enabled, on, false);
            state.shell.ribbon_rects.push((cmd.id, resp.rect, enabled));
            let resp = resp.on_hover_text(tip);
            if resp.clicked() {
                state.shell.clicked.push(cmd.id);
                commands::run(state, id);
            }
        }
        Item::Menu {
            id,
            label,
            glyph,
            size,
            body,
            active,
        } => {
            let on = active(state);
            let resp = button(ui, state, label, *glyph, *size, true, on, true);
            state.shell.ribbon_rects.push((id, resp.rect, true));
            if resp.clicked() {
                state.shell.clicked.push(id);
            }
            egui::Popup::menu(&resp).show(|ui| {
                ui.set_min_width(200.0);
                body(ui, state);
            });
        }
        Item::Custom(f) => {
            ui.vertical(|ui| {
                ui.set_min_height(LG.y);
                f(ui, state);
            });
        }
    }
}

/// A ribbon button. Large: icon above the label; small: icon left of it.
#[allow(clippy::too_many_arguments)]
fn button(
    ui: &mut Ui,
    state: &AppState,
    label: &str,
    glyph: char,
    size: Size,
    enabled: bool,
    on: bool,
    menu: bool,
) -> egui::Response {
    let s = state.shell.shades;
    let (font, icon_size) = match size {
        Size::Lg => (FontId::proportional(11.0), 21.0),
        Size::Sm => (FontId::proportional(11.5), 14.0),
    };
    // A drop-down gets a chevron from the icon font (Segoe UI has no ▾).
    let mut job = egui::text::LayoutJob::default();
    let format = |font_id| egui::TextFormat {
        font_id,
        color: egui::Color32::PLACEHOLDER,
        ..Default::default()
    };
    job.append(label, 0.0, format(font.clone()));
    if menu {
        job.append(
            &icon::CHEVRON_DOWN.to_string(),
            3.0,
            format(theme::icon_font(font.size - 2.0, state.shell.fonts_ready())),
        );
    }
    if size == Size::Lg {
        job.wrap.max_width = LG.x + 8.0;
        job.halign = egui::Align::Center;
    }
    let galley = ui.painter().layout_job(job);
    let rect_size = match size {
        Size::Lg => Vec2::new(LG.x.max(galley.size().x + 6.0), LG.y),
        Size::Sm => Vec2::new(5.0 + icon_size + 6.0 + galley.size().x + 8.0, SM_HEIGHT),
    };
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(rect_size, sense);
    let painter = ui.painter();
    if on {
        painter.rect(
            rect,
            CornerRadius::same(5),
            s.accent_dim,
            Stroke::new(1.0, s.accent.gamma_multiply(0.5)),
            StrokeKind::Inside,
        );
    } else if enabled && resp.hovered() {
        painter.rect(
            rect,
            CornerRadius::same(5),
            s.panel2,
            Stroke::new(1.0, s.border),
            StrokeKind::Inside,
        );
    }
    let alpha = if enabled { 1.0 } else { 0.35 };
    let icon_color = if on { s.accent } else { s.heading }.gamma_multiply(alpha);
    let text_color = if on { s.accent_text } else { s.text }.gamma_multiply(alpha);
    let icon_font = theme::icon_font(icon_size, state.shell.fonts_ready());
    match size {
        Size::Lg => {
            painter.text(
                Pos2::new(rect.center().x, rect.top() + 6.0 + icon_size / 2.0),
                Align2::CENTER_CENTER,
                glyph,
                icon_font,
                icon_color,
            );
            let pos = Pos2::new(
                rect.center().x - galley.rect.center().x,
                rect.top() + 6.0 + icon_size + 4.0,
            );
            painter.galley(pos, galley, text_color);
        }
        Size::Sm => {
            painter.text(
                Pos2::new(rect.left() + 5.0 + icon_size / 2.0, rect.center().y),
                Align2::CENTER_CENTER,
                glyph,
                icon_font,
                icon_color,
            );
            let pos = Pos2::new(
                rect.left() + 5.0 + icon_size + 6.0,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(pos, galley, text_color);
        }
    }
    resp
}

/// A command inside a drop-down: label, key on the right; runs and closes.
pub fn menu_cmd(ui: &mut Ui, state: &mut AppState, id: &'static str, label: &str) {
    let enabled = commands::enabled(state, id);
    let key = state.shell.key_of(id);
    let on = commands::active(state, id);
    let text = if on {
        format!("\u{2713} {label}")
    } else {
        label.to_owned()
    };
    let mut button = egui::Button::new(text);
    if !key.is_empty() {
        button = button.shortcut_text(key);
    }
    if ui.add_enabled(enabled, button).clicked() {
        commands::run(state, id);
        ui.close();
    }
}

fn recent_menu(ui: &mut Ui, state: &mut AppState) {
    let profile = state.active_profile_id().map(str::to_owned);
    let recent: Vec<String> = state
        .config
        .recent_maps
        .iter()
        .filter(|r| Some(&r.profile) == profile.as_ref())
        .map(|r| r.path.clone())
        .collect();
    if recent.is_empty() {
        ui.weak("No maps opened yet.");
    }
    for path in recent {
        let short = path
            .strip_prefix("data\\global\\tiles\\")
            .unwrap_or(&path)
            .replace('\\', "/");
        if ui.button(short).on_hover_text(&path).clicked() {
            state.open_map(&GamePath::new(&path));
            ui.close();
        }
    }
}

fn profile_menu(ui: &mut Ui, state: &mut AppState) {
    let current = state.active_profile_id().map(str::to_owned);
    let mut chosen = None;
    for p in &state.profiles {
        let on = current.as_deref() == Some(p.id.as_str());
        if ui.selectable_label(on, &p.name).clicked() && !on {
            chosen = Some(p.id.clone());
        }
    }
    if state.profiles.is_empty() {
        ui.weak(format!(
            "No profiles: add *.toml to {}",
            state.paths.profiles.display()
        ));
    }
    if let Some(id) = chosen {
        state.open_profile_by_id(&id);
        ui.close();
    }
}

fn import_menu(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "file.importPackage", "Map package...");
    menu_cmd(ui, state, "file.importDs1Dt1", "DS1 with its DT1s...");
    menu_cmd(ui, state, "file.importIni", "DS1Edit ini mappings...");
}

fn export_menu(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "file.exportPackage", "Map package (.zip)...");
    ui.separator();
    export_images(ui, state);
}

fn saves_menu(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "file.openSaves", "Open saves folder");
    let changed = state.edit.changed;
    menu_cmd(
        ui,
        state,
        "file.markPacked",
        &format!("Mark as packed ({changed})"),
    );
    ui.weak(format!("CHANGED.txt: {changed} file(s)"));
}

fn export_images(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "export.png", "Map as PNG...");
    menu_cmd(ui, state, "export.selectionPng", "Selection as PNG...");
    menu_cmd(ui, state, "view.snapshot", "Copy the view to the clipboard");
}

fn game_size_menu(ui: &mut Ui, state: &mut AppState) {
    let now = (
        state.config.ui.studio.game_width,
        state.config.ui.studio.game_height,
    );
    for (w, h) in super::studio::views::GAME_SIZES {
        if ui
            .selectable_label(now == (w, h), format!("{w} \u{d7} {h}"))
            .clicked()
        {
            state.config.ui.studio.game_width = w;
            state.config.ui.studio.game_height = h;
            state.config_dirty = true;
            ui.close();
        }
    }
}

fn flags_menu(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "flags.setHidden", "Set hidden");
    menu_cmd(ui, state, "flags.clearHidden", "Clear hidden");
    menu_cmd(ui, state, "flags.setUnwalkable", "Set unwalkable");
    menu_cmd(ui, state, "flags.clearUnwalkable", "Clear unwalkable");
}

fn select_menu(ui: &mut Ui, state: &mut AppState) {
    menu_cmd(ui, state, "tool.lasso", "Lasso");
    menu_cmd(ui, state, "tool.selectById", "Select by tile ID");
    menu_cmd(ui, state, "tool.selectIdentical", "Select identical");
    ui.separator();
    ui.weak("Selection tools look at");
    menu_cmd(ui, state, "select.scopeActive", "The active layer");
    menu_cmd(ui, state, "select.scopeVisible", "Every visible layer");
    ui.separator();
    ui.weak(crate::edit::tools::SELECT_MODS_HINT);
}

fn theme_menu(ui: &mut Ui, state: &mut AppState) {
    let mut themes = theme::builtin_themes();
    themes.extend(
        state
            .config
            .ui
            .custom_themes
            .iter()
            .filter_map(theme::CustomTheme::to_theme),
    );
    for t in themes {
        let on = state.config.ui.theme == t.id;
        if ui.selectable_label(on, &t.name).clicked() && !on {
            state.config.ui.theme = t.id.clone();
            state.config_dirty = true;
            ui.close();
        }
    }
    ui.separator();
    menu_cmd(ui, state, "file.preferences", "Custom themes...");
}

/// Paint / erase: the random mix, the brush size and the value Paint
/// writes (the Freehand / Rectangle / Fill area modes are registry buttons).
fn paint_group(ui: &mut Ui, state: &mut AppState) {
    let has_doc = state.active.is_some();
    let s = state.shell.shades;
    ui.add_enabled_ui(has_doc, |ui| {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                super::studio::paint::mix_line(ui, state);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Brush").size(11.5).color(s.muted));
                    ui.add(
                        egui::DragValue::new(&mut state.edit.brush)
                            .range(1..=16)
                            .suffix(" cells"),
                    )
                    .on_hover_text("Brush size of Paint and Erase");
                });
            });
            let layer = state
                .edit
                .current_layer
                .or_else(|| state.active_doc().map(|d| d.active_layer));
            let current = state.edit.current;
            match layer {
                Some(layer) if !layer.is_empty_value(current) => {
                    super::thumbs::show(ui, state, layer, current, false).on_hover_text(format!(
                        "Paint writes {} cell {:#010x}, orientation {}",
                        crate::doc::layer_label(layer),
                        current.cell,
                        current.orientation
                    ));
                }
                _ => {
                    ui.label(
                        egui::RichText::new("No value yet:\npick a tile")
                            .size(11.0)
                            .color(s.muted),
                    );
                }
            }
        });
    });
}

/// "Active layer" and its picker.
fn active_layer(ui: &mut Ui, state: &mut AppState) {
    let s = state.shell.shades;
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Active layer")
            .size(11.5)
            .color(s.muted),
    );
    let Some(doc) = state.active_doc_mut() else {
        ui.add_enabled(false, egui::Button::new("No map"));
        return;
    };
    let keys = doc.keys();
    let current = LayerKey::from_ref(doc.active_layer);
    egui::ComboBox::from_id_salt("ribbon_active_layer")
        .selected_text(current.label())
        .width(120.0)
        .show_ui(ui, |ui| {
            for k in keys {
                if ui.selectable_label(k == current, k.label()).clicked() {
                    doc.active_layer = k.to_ref();
                }
            }
        });
}

/// Map right-click menu, built from the registry.
pub fn map_context_menu(ui: &mut Ui, state: &mut AppState) {
    for (id, label) in [
        ("edit.cut", "Cut"),
        ("edit.copy", "Copy"),
        ("edit.paste", "Paste"),
        ("edit.delete", "Delete"),
    ] {
        menu_cmd(ui, state, id, label);
    }
    ui.separator();
    menu_cmd(
        ui,
        state,
        "edit.selectAll",
        "Select all on the active layer",
    );
    menu_cmd(ui, state, "edit.deselect", "Deselect");
    menu_cmd(ui, state, "edit.moveToLayer", "Move selection to layer...");
    ui.menu_button("Selection flags", |ui| flags_menu(ui, state));
    menu_cmd(ui, state, "edit.replace", "Find & replace tiles...");
    menu_cmd(
        ui,
        state,
        "export.selectionPng",
        "Export the selection as PNG...",
    );
    ui.separator();
    menu_cmd(ui, state, "view.fit", "Fit map");
    menu_cmd(ui, state, "view.grid", "Grid");
    menu_cmd(ui, state, "app.commands", "Find a command...");
}

// ---- scripted clicks --------------------------------------------------------

/// Where a scripted "click every ribbon button" stands.
pub struct ClickAll {
    tabs: VecDeque<&'static str>,
    queue: VecDeque<&'static str>,
    step: ClickStep,
    /// Buttons clicked (and seen to react), in order.
    pub done: Vec<&'static str>,
    /// Disabled or external buttons that were left alone.
    pub skipped: Vec<&'static str>,
}

#[derive(Clone, Copy)]
enum ClickStep {
    NextTab,
    /// Waiting for the tab to be drawn, then list its buttons.
    Collect(u32),
    Next,
    /// Waiting for the events and the reaction; then clean up.
    Check(&'static str, usize, u32),
}

impl ClickAll {
    pub fn new() -> ClickAll {
        ClickAll {
            tabs: tab_ids().into(),
            queue: VecDeque::new(),
            step: ClickStep::NextTab,
            done: Vec::new(),
            skipped: Vec::new(),
        }
    }
}

/// The pointer events of one click at `pos`, one batch per frame.
pub fn click_events(pos: Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    vec![
        vec![egui::Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
        vec![egui::Event::PointerMoved(Pos2::new(-50.0, -50.0))],
    ]
}

fn rect_of(state: &AppState, id: &str) -> Option<(Rect, bool)> {
    state
        .shell
        .ribbon_rects
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, r, e)| (*r, *e))
}

/// One frame of the scripted run. `Ok(true)` when every tab is done.
pub fn click_all_step(state: &mut AppState) -> Result<bool, String> {
    let Some(mut run) = state.shell.click_all.take() else {
        return Ok(true);
    };
    let result = advance(state, &mut run);
    if !matches!(result, Ok(true)) {
        state.shell.click_all = Some(run);
    } else {
        state.info(format!(
            "click-ribbon: {} button(s) clicked, {} skipped ({})",
            run.done.len(),
            run.skipped.len(),
            run.skipped.join(", ")
        ));
        state.shell.click_all = Some(run);
    }
    result
}

fn advance(state: &mut AppState, run: &mut ClickAll) -> Result<bool, String> {
    if !state.shell.inject.is_empty() {
        return Ok(false);
    }
    match run.step {
        ClickStep::NextTab => {
            let Some(tab) = run.tabs.pop_front() else {
                return Ok(true);
            };
            let id = tab_rect_id(tab);
            let (rect, _) = rect_of(state, id).ok_or_else(|| format!("tab {tab} is not drawn"))?;
            state.shell.inject.extend(click_events(rect.center()));
            run.step = ClickStep::Collect(0);
            run.queue.clear();
            let _ = tab;
        }
        ClickStep::Collect(frames) => {
            if frames < 3 {
                run.step = ClickStep::Collect(frames + 1);
                return Ok(false);
            }
            for (id, _, enabled) in state.shell.ribbon_rects.clone() {
                if id.starts_with("tab.") || id.starts_with("top.") {
                    continue;
                }
                let external = commands::find(id).is_some_and(|c| c.external);
                if enabled && !external {
                    run.queue.push_back(id);
                } else {
                    run.skipped.push(id);
                }
            }
            run.step = ClickStep::Next;
        }
        ClickStep::Next => {
            let Some(id) = run.queue.pop_front() else {
                run.step = ClickStep::NextTab;
                return Ok(false);
            };
            match rect_of(state, id) {
                Some((rect, true)) => {
                    let before = state.shell.clicked.len();
                    state.shell.inject.extend(click_events(rect.center()));
                    run.step = ClickStep::Check(id, before, 0);
                }
                // Disabled by an earlier click (Undo after the last undo...).
                _ => run.skipped.push(id),
            }
        }
        ClickStep::Check(id, before, frames) => {
            if frames < 2 {
                run.step = ClickStep::Check(id, before, frames + 1);
                return Ok(false);
            }
            if !state.shell.clicked[before.min(state.shell.clicked.len())..].contains(&id) {
                return Err(format!("ribbon button {id} did not react to a click"));
            }
            run.done.push(id);
            super::shell::close_dialogs(state);
            run.step = ClickStep::Next;
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ribbon_shows_only_available_commands() {
        for id in ribbon_commands() {
            let c = commands::find(id);
            assert!(c.is_some(), "{id} is not registered");
            assert!(
                c.is_some_and(|c| c.available),
                "{id} is shown but not available"
            );
        }
        assert_eq!(tab_ids(), ["home", "view", "map", "game", "diag", "help"]);
    }
}
