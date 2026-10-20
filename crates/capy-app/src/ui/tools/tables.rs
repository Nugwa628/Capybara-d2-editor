//! The table editor: a spreadsheet over `TxtDoc` with column help. Cells
//! are edited in place, rows duplicated, inserted and deleted; every edit is
//! undoable. Unedited rows keep their exact bytes (the writer is lossless).
//! Save writes the table to `capybara\saves\<profile>\data\global\excel\`.

use bevy_egui::egui::{self, Color32, Sense, Ui};
use d2_formats::txt::TxtDoc;

use super::column_help::column_help;
use super::io;
use crate::state::AppState;

/// The tables map makers touch most, listed first.
const FAVOURITES: &[&str] = &[
    "lvlprest.txt",
    "lvltypes.txt",
    "levels.txt",
    "lvlwarp.txt",
    "lvlmaze.txt",
    "lvlsub.txt",
    "objects.txt",
    "objgroup.txt",
    "monpreset.txt",
    "superuniques.txt",
    "cubemain.txt",
    "misc.txt",
    "automap.txt",
    "monstats.txt",
    "armor.txt",
    "weapons.txt",
    "skills.txt",
    "itemtypes.txt",
];

const MAX_UNDO: usize = 30;

#[derive(Default)]
pub struct TablesUi {
    pub name: String,
    pub doc: Option<TxtDoc>,
    undo: Vec<TxtDoc>,
    redo: Vec<TxtDoc>,
    pub dirty: bool,
    list: Vec<String>,
    scanned: bool,
    filter: String,
    open_name: String,
    pub sel: Option<(usize, usize)>,
    editing: Option<(usize, usize, String)>,
    search: String,
    pub msg: String,
    scroll_to: Option<usize>,
}

impl TablesUi {
    fn push_undo(&mut self) {
        if let Some(d) = &self.doc {
            self.undo.push(d.clone());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
        }
    }

    /// Sets a cell (one undo step). `col` is a header name.
    pub fn set_cell(&mut self, row: usize, col: &str, value: &str) -> Result<(), String> {
        let c = self
            .doc
            .as_ref()
            .and_then(|d| d.column(col))
            .ok_or_else(|| format!("{} has no column {col}", self.name))?;
        self.set_cell_at(row, c, value)
    }

    pub fn set_cell_at(&mut self, row: usize, col: usize, value: &str) -> Result<(), String> {
        let same = self
            .doc
            .as_ref()
            .and_then(|d| d.cell(row, col))
            .is_some_and(|v| v == value);
        if same {
            return Ok(());
        }
        self.push_undo();
        let doc = self.doc.as_mut().ok_or("No table is open")?;
        if let Err(e) = doc.set_cell(row, col, value) {
            self.undo.pop();
            return Err(e.to_string());
        }
        Ok(())
    }

    pub fn undo(&mut self) {
        if let (Some(prev), Some(cur)) = (self.undo.pop(), self.doc.take()) {
            self.redo.push(cur);
            self.doc = Some(prev);
            self.dirty = true;
        }
    }

    pub fn redo(&mut self) {
        if let (Some(next), Some(cur)) = (self.redo.pop(), self.doc.take()) {
            self.undo.push(cur);
            self.doc = Some(next);
            self.dirty = true;
        }
    }
}

/// Opens a table by file name (`levels.txt`).
pub fn open_table(state: &mut AppState, name: &str) -> Result<(), String> {
    let name = name.trim().to_ascii_lowercase();
    let name = if name.ends_with(".txt") {
        name
    } else {
        format!("{name}.txt")
    };
    let doc = io::read_txt(state, &name)?;
    let t = &mut state.tools.tables;
    t.name = name;
    t.doc = Some(doc);
    t.undo.clear();
    t.redo.clear();
    t.dirty = false;
    t.sel = None;
    t.editing = None;
    t.msg.clear();
    Ok(())
}

/// Writes the open table to the saves.
pub fn save_table(state: &mut AppState) -> Result<(), String> {
    let (name, doc) = {
        let t = &state.tools.tables;
        (t.name.clone(), t.doc.clone().ok_or("No table is open")?)
    };
    io::save_txt(state, &name, &doc)?;
    if matches!(
        name.as_str(),
        "levels.txt" | "lvlprest.txt" | "lvltypes.txt"
    ) {
        state.rebuild_catalog();
    }
    state.tools.tables.dirty = false;
    state.tools.tables.msg = format!("Saved {name}");
    Ok(())
}

fn scan(state: &mut AppState) {
    let mut names: Vec<String> = Vec::new();
    if let Some(session) = &state.session {
        for layer in session.overlay.layers() {
            if let Ok(list) = layer.list() {
                for p in list {
                    let s = p.as_str();
                    if let Some(rest) = s.strip_prefix("data\\global\\excel\\")
                        && rest.ends_with(".txt")
                        && !rest.contains('\\')
                    {
                        names.push(rest.to_owned());
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    let rank = |n: &str| FAVOURITES.iter().position(|f| *f == n).unwrap_or(1000);
    names.sort_by_key(|n| rank(n));
    state.tools.tables.list = names;
    state.tools.tables.scanned = true;
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let mut t = std::mem::take(&mut state.tools.tables);
    if t.list.is_empty() {
        t.list = FAVOURITES.iter().map(|s| (*s).to_owned()).collect();
    }
    ui.horizontal(|ui| {
        ui.label("Table:");
        let mut pick: Option<String> = None;
        egui::ComboBox::from_id_salt("tbl_pick")
            .width(180.0)
            .selected_text(if t.name.is_empty() {
                "(none)"
            } else {
                t.name.as_str()
            })
            .show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut t.filter).hint_text("filter"));
                let f = t.filter.to_ascii_lowercase();
                for n in t
                    .list
                    .iter()
                    .filter(|n| f.is_empty() || n.contains(&f))
                    .take(400)
                {
                    if ui.selectable_label(*n == t.name, n.as_str()).clicked() {
                        pick = Some(n.clone());
                    }
                }
            });
        if ui
            .small_button("All tables")
            .on_hover_text("List every excel table the profile has")
            .clicked()
        {
            state.tools.tables = std::mem::take(&mut t);
            scan(state);
            t = std::mem::take(&mut state.tools.tables);
        }
        ui.add(
            egui::TextEdit::singleline(&mut t.open_name)
                .hint_text("or a name")
                .desired_width(100.0),
        );
        if ui.small_button("Open").clicked() && !t.open_name.trim().is_empty() {
            pick = Some(t.open_name.clone());
        }
        if let Some(p) = pick {
            state.tools.tables = std::mem::take(&mut t);
            if let Err(e) = open_table(state, &p) {
                state.tools.tables.msg = e;
            }
            t = std::mem::take(&mut state.tools.tables);
        }
    });
    if t.doc.is_none() {
        super::message(ui, &t.msg);
        ui.label("Pick a table to edit. Saved copies go to capybara\\saves.");
        state.tools.tables = t;
        return;
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(t.dirty, egui::Button::new("Save"))
            .on_hover_text("Write to the profile's saves, with a CHANGED.txt entry")
            .clicked()
        {
            state.tools.tables = std::mem::take(&mut t);
            if let Err(e) = save_table(state) {
                state.tools.tables.msg = e;
            }
            t = std::mem::take(&mut state.tools.tables);
        }
        if ui
            .add_enabled(!t.undo.is_empty(), egui::Button::new("Undo"))
            .clicked()
        {
            t.undo();
        }
        if ui
            .add_enabled(!t.redo.is_empty(), egui::Button::new("Redo"))
            .clicked()
        {
            t.redo();
        }
        ui.separator();
        let at = t.sel.map(|s| s.0);
        if ui
            .add_enabled(at.is_some(), egui::Button::new("Duplicate row"))
            .clicked()
            && let Some(r) = at
        {
            t.push_undo();
            if let Some(d) = t.doc.as_mut() {
                match d.duplicate_row(r) {
                    Ok(()) => t.sel = Some((r + 1, t.sel.map_or(0, |s| s.1))),
                    Err(e) => t.msg = e.to_string(),
                }
            }
        }
        if ui
            .add_enabled(at.is_some(), egui::Button::new("Insert blank row"))
            .clicked()
            && let Some(r) = at
        {
            t.push_undo();
            if let Some(d) = t.doc.as_mut() {
                let blanks = vec![""; d.headers().len()];
                if let Err(e) = d.insert_row(r + 1, &blanks) {
                    t.msg = e.to_string();
                }
            }
        }
        if ui
            .add_enabled(at.is_some(), egui::Button::new("Delete row"))
            .clicked()
            && let Some(r) = at
        {
            t.push_undo();
            if let Some(d) = t.doc.as_mut()
                && let Err(e) = d.delete_row(r)
            {
                t.msg = e.to_string();
            }
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut t.search)
                .hint_text("find")
                .desired_width(120.0),
        );
        if ui.small_button("Find next").clicked() && !t.search.is_empty() {
            let needle = t.search.to_ascii_lowercase();
            if let Some(d) = t.doc.as_ref() {
                let start = t.sel.map_or(0, |s| s.0 + 1);
                let n = d.row_count();
                let hit = (0..n).map(|i| (start + i) % n.max(1)).find(|&r| {
                    d.row_cells(r).is_some_and(|cells| {
                        cells.iter().any(|c| {
                            String::from_utf8_lossy(c)
                                .to_ascii_lowercase()
                                .contains(&needle)
                        })
                    })
                });
                if let Some(r) = hit {
                    t.sel = Some((r, t.sel.map_or(0, |s| s.1)));
                    t.scroll_to = Some(r);
                }
            }
        }
    });
    if t.dirty {
        ui.colored_label(Color32::from_rgb(230, 190, 90), "Unsaved changes");
    }
    super::message(ui, &t.msg);
    grid(ui, &mut t);
    // Help for the selected column.
    if let (Some(d), Some((_, c))) = (t.doc.as_ref(), t.sel) {
        let headers = d.headers();
        if let Some(h) = headers.get(c) {
            let help = column_help(&t.name, h).unwrap_or("No help for this column yet.");
            ui.add(egui::Label::new(format!("{h}: {help}")).wrap());
        }
    }
    state.tools.tables = t;
}

const ROW_H: f32 = 20.0;
const COL_W: f32 = 104.0;
const NUM_W: f32 = 46.0;

fn grid(ui: &mut Ui, t: &mut TablesUi) {
    let Some(doc) = t.doc.as_ref() else { return };
    let headers = doc.headers();
    let (ncols, nrows) = (headers.len(), doc.row_count());
    let total = egui::vec2(NUM_W + COL_W * ncols as f32, ROW_H * (nrows as f32 + 1.0));
    let mut commit: Option<(usize, usize, String)> = None;
    let mut select: Option<(usize, usize)> = None;
    let mut start_edit: Option<(usize, usize, String)> = None;
    let mut cancel = false;
    let scroll_to = t.scroll_to.take();
    let mut area = egui::ScrollArea::both()
        .id_salt("tbl_scroll")
        .auto_shrink([false, false])
        .max_height((ui.available_height() - 40.0).max(120.0));
    if let Some(r) = scroll_to {
        area = area.vertical_scroll_offset((r as f32 * ROW_H - 80.0).max(0.0));
    }
    area.show_viewport(ui, |ui, vp| {
        let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
        let painter = ui.painter_at(rect.intersect(ui.clip_rect()));
        let visuals = ui.visuals().clone();
        let first = ((vp.min.y / ROW_H).floor() as usize).saturating_sub(1);
        let last = (((vp.max.y / ROW_H).ceil() as usize) + 1).min(nrows + 1);
        let c0 = (((vp.min.x - NUM_W) / COL_W).floor().max(0.0)) as usize;
        let c1 = ((((vp.max.x - NUM_W) / COL_W).ceil().max(0.0)) as usize + 1).min(ncols);
        for r in first..last {
            if r >= nrows {
                break;
            }
            let y = rect.top() + (r as f32 + 1.0) * ROW_H;
            let row_rect =
                egui::Rect::from_min_size(egui::pos2(rect.left(), y), egui::vec2(NUM_W, ROW_H));
            painter.text(
                row_rect.right_center() - egui::vec2(4.0, 0.0),
                egui::Align2::RIGHT_CENTER,
                format!("{r}"),
                egui::FontId::monospace(10.0),
                visuals.weak_text_color(),
            );
            for c in c0..c1 {
                let cell_rect = egui::Rect::from_min_size(
                    egui::pos2(rect.left() + NUM_W + c as f32 * COL_W, y),
                    egui::vec2(COL_W, ROW_H),
                );
                let is_sel = t.sel == Some((r, c));
                let editing_here = t.editing.as_ref().is_some_and(|e| e.0 == r && e.1 == c);
                if editing_here {
                    let buf = &mut t.editing.as_mut().expect("editing").2;
                    let resp = ui.put(
                        cell_rect,
                        egui::TextEdit::singleline(buf).margin(egui::Margin::symmetric(3, 1)),
                    );
                    resp.request_focus();
                    if resp.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        } else {
                            commit = Some((r, c, buf.clone()));
                        }
                    }
                    continue;
                }
                let value = doc.cell(r, c).unwrap_or_default();
                if is_sel {
                    painter.rect_filled(cell_rect, 0.0, visuals.selection.bg_fill);
                } else if r % 2 == 1 {
                    painter.rect_filled(cell_rect, 0.0, visuals.faint_bg_color);
                }
                painter.rect_stroke(
                    cell_rect,
                    0.0,
                    egui::Stroke::new(0.5, visuals.widgets.noninteractive.bg_stroke.color),
                    egui::StrokeKind::Inside,
                );
                painter
                    .with_clip_rect(cell_rect.shrink2(egui::vec2(1.0, 0.0)))
                    .text(
                        cell_rect.left_center() + egui::vec2(4.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        &value,
                        egui::FontId::proportional(12.0),
                        visuals.text_color(),
                    );
                let resp = ui.interact(cell_rect, ui.id().with(("cell", r, c)), Sense::click());
                if resp.double_clicked() {
                    start_edit = Some((r, c, value));
                } else if resp.clicked() {
                    select = Some((r, c));
                }
            }
        }
        // The header stays at the top of the view.
        let top = rect.top() + vp.min.y.max(0.0);
        let header =
            egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(total.x, ROW_H));
        painter.rect_filled(header, 0.0, visuals.extreme_bg_color);
        painter.text(
            header.left_center() + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            "#",
            egui::FontId::monospace(10.0),
            visuals.weak_text_color(),
        );
        for (c, head) in headers.iter().enumerate().take(c1).skip(c0) {
            let cell_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + NUM_W + c as f32 * COL_W, top),
                egui::vec2(COL_W, ROW_H),
            );
            painter.with_clip_rect(cell_rect).text(
                cell_rect.left_center() + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                head,
                egui::FontId::proportional(12.0),
                visuals.strong_text_color(),
            );
            let resp = ui.interact(cell_rect, ui.id().with(("head", c)), Sense::hover());
            if let Some(h) = column_help(&t.name, head) {
                resp.on_hover_text(h);
            } else {
                resp.on_hover_text(head);
            }
        }
    });
    if let Some(s) = select {
        t.sel = Some(s);
        t.editing = None;
    }
    if let Some(e) = start_edit {
        t.sel = Some((e.0, e.1));
        t.editing = Some(e);
    }
    if cancel {
        t.editing = None;
    }
    if let Some((r, c, v)) = commit {
        t.editing = None;
        if let Err(e) = t.set_cell_at(r, c, &v) {
            t.msg = e;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_edits_undo_and_keep_other_rows() {
        let src = b"Name\tId\r\nA\t1\r\nB\t2\r\n";
        let mut t = TablesUi {
            doc: Some(TxtDoc::parse(src)),
            name: "x.txt".into(),
            ..TablesUi::default()
        };
        t.set_cell(1, "Id", "7").expect("set");
        assert!(t.dirty);
        assert_eq!(
            t.doc.as_ref().expect("doc").to_bytes(),
            b"Name\tId\r\nA\t1\r\nB\t7\r\n"
        );
        t.undo();
        assert_eq!(t.doc.as_ref().expect("doc").to_bytes(), src);
        t.redo();
        assert_eq!(
            t.doc.as_ref().expect("doc").cell(1, 1).as_deref(),
            Some("7")
        );
        assert!(t.set_cell(0, "Nope", "1").is_err());
    }
}
