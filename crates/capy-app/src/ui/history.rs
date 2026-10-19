//! The History panel: the active map's snapshot timeline (newest first)
//! with label / checkpoint / restore / delete, the diff summary against the
//! selected snapshot, and the milestone log. Also the panel's prompts.

use bevy_egui::egui::{self, Color32, Context, Id, RichText, Ui};
use capy_edit::LayerRef;
use capy_history::{Snapshot, SnapshotId, SnapshotKind};

use crate::doc::layer_label;
use crate::history::overlay::Change;
use crate::history::{HistoryPrompt, display_name, human_size, kind_label, local_time};
use crate::state::AppState;

fn change_color(change: Change) -> Color32 {
    let [r, g, b, _] = change.rgba();
    Color32::from_rgb(r, g, b)
}

fn kind_color(kind: &SnapshotKind) -> Color32 {
    match kind {
        SnapshotKind::Checkpoint { .. } => Color32::from_rgb(120, 190, 255),
        SnapshotKind::Save => Color32::from_rgb(120, 210, 130),
        SnapshotKind::BeforeRestore => Color32::from_rgb(230, 170, 90),
        _ => Color32::from_gray(190),
    }
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let Some(index) = state.active else {
        ui.weak("Open a map to see its snapshots.");
        ui.separator();
        milestones(ui, state);
        return;
    };
    let (doc_id, name, dirty, viewing) = {
        let d = &state.docs[index];
        (
            d.id,
            d.path.file_name().to_owned(),
            d.is_dirty(),
            d.snapshot_view,
        )
    };

    ui.horizontal_wrapped(|ui| {
        ui.strong(&name);
        if dirty {
            ui.weak("(unsaved changes)");
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Checkpoint...")
            .on_hover_text("Record the map as it is now under a name, without saving it")
            .clicked()
        {
            state.history.prompt = Some(HistoryPrompt::Checkpoint {
                name: String::new(),
            });
        }
        if ui
            .add_enabled(dirty, egui::Button::new("Autosave now"))
            .on_hover_text("Record every map with unsaved changes in history now")
            .on_disabled_hover_text("No unsaved changes")
            .clicked()
        {
            let n = state.autosave_now();
            if n == 0 {
                state.notify("Nothing new to autosave");
            }
        }
        if ui
            .button("Milestone...")
            .on_hover_text(r"Commit capybara\saves\ (every profile) as a git milestone")
            .clicked()
        {
            state.history.prompt = Some(HistoryPrompt::Milestone {
                message: String::new(),
            });
        }
        if ui.button("Refresh").clicked() {
            state.history.invalidate();
            state.refresh_milestones();
        }
    });
    if let Some(at) = &state.history.last_autosave {
        ui.weak(format!("autosaved {at}"));
    }
    if state.history.autosave_in_flight {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak("autosaving...");
        });
    }

    let timeline = state.timeline_of(index);
    if let Some(e) = state.history.list_error.clone() {
        ui.colored_label(Color32::from_rgb(235, 110, 90), format!("History: {e}"));
    }
    let selected = state.history.selected_for(doc_id);
    ui.separator();
    ui.label(format!("{} snapshot(s), newest first", timeline.len()));
    if timeline.is_empty() {
        ui.weak("Saves, autosaves and checkpoints of this map appear here.");
    }
    let mut pick: Option<SnapshotId> = None;
    let mut action: Option<HistoryPrompt> = None;
    // Both directions: a wide grid must not widen the whole panel.
    egui::ScrollArea::both()
        .id_salt("history_timeline")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("history_grid")
                .num_columns(4)
                .striped(true)
                .spacing([10.0, 3.0])
                .show(ui, |ui| {
                    ui.weak("Kind");
                    ui.weak("Name / label");
                    ui.weak("Time");
                    ui.weak("Size");
                    ui.end_row();
                    for s in &timeline {
                        let is_sel = selected == Some(s.id);
                        let text = RichText::new(kind_label(&s.kind)).color(kind_color(&s.kind));
                        let resp = ui.selectable_label(is_sel, text).on_hover_text(
                            "Click to compare with the map; right-click for actions",
                        );
                        if resp.clicked() {
                            pick = Some(s.id);
                        }
                        resp.context_menu(|ui| {
                            if let Some(a) = row_actions(ui, s) {
                                action = Some(a);
                                ui.close();
                            }
                        });
                        let mut label = display_name(s).unwrap_or("").to_owned();
                        if let (SnapshotKind::Checkpoint { .. }, Some(l)) = (&s.kind, &s.label) {
                            label = format!("{label} ({l})");
                        }
                        ui.label(label);
                        ui.label(local_time(s.created_unix_ms, "%b %d %H:%M:%S"))
                            .on_hover_text(local_time(s.created_unix_ms, "%Y-%m-%d %H:%M:%S %Z"));
                        ui.label(human_size(s.size));
                        ui.end_row();
                    }
                });
        });
    if let Some(id) = pick
        && let Err(e) = state.select_snapshot(id)
    {
        state.error(format!("Cannot read that snapshot: {e}"));
    }

    if let Some(s) = selected.and_then(|id| timeline.iter().find(|s| s.id == id)) {
        ui.horizontal_wrapped(|ui| {
            if let Some(a) = row_actions(ui, s) {
                action = Some(a);
            }
        });
        ui.separator();
        compare(ui, state, s, viewing == Some(s.id));
    }
    if action.is_some() {
        state.history.prompt = action;
    }
    ui.separator();
    milestones(ui, state);
}

fn row_prompts(s: &Snapshot) -> [(&'static str, HistoryPrompt); 4] {
    [
        (
            "Rename / label...",
            HistoryPrompt::Label {
                id: s.id,
                text: s.label.clone().unwrap_or_default(),
            },
        ),
        (
            if matches!(s.kind, SnapshotKind::Checkpoint { .. }) {
                "Rename checkpoint..."
            } else {
                "Make checkpoint..."
            },
            HistoryPrompt::MakeCheckpoint {
                id: s.id,
                name: display_name(s).unwrap_or("").to_owned(),
            },
        ),
        ("Restore...", HistoryPrompt::Restore { id: s.id }),
        ("Delete...", HistoryPrompt::Delete { id: s.id }),
    ]
}

fn row_actions(ui: &mut Ui, s: &Snapshot) -> Option<HistoryPrompt> {
    let mut out = None;
    for (label, prompt) in row_prompts(s) {
        if ui.button(label).clicked() {
            out = Some(prompt);
        }
    }
    out
}

fn compare(ui: &mut Ui, state: &mut AppState, s: &Snapshot, viewing: bool) {
    let what = match display_name(s) {
        Some(n) => format!("{} \"{n}\"", kind_label(&s.kind)),
        None => kind_label(&s.kind).to_owned(),
    };
    ui.label(RichText::new(format!("Changes since {what}")).strong());
    let Some(index) = state.active else { return };
    let doc_id = state.docs[index].id;
    if let Some(e) = state.history.compare_error(doc_id) {
        ui.colored_label(Color32::from_rgb(235, 110, 90), e.to_owned());
        return;
    }
    ui.checkbox(
        &mut state.history.overlay_visible,
        "Show changes on the map",
    );
    let mut show = viewing;
    if ui
        .checkbox(&mut show, "Show snapshot (read-only)")
        .on_hover_text("Draw the snapshot in place of the map, to flip between then and now")
        .changed()
        && let Err(e) = state.set_show_snapshot(show)
    {
        state.error(e);
    }
    if viewing {
        ui.colored_label(
            Color32::from_rgb(120, 190, 255),
            "Showing the snapshot. Untick to edit the map again.",
        );
    }
    ui.horizontal(|ui| {
        for c in Change::ALL {
            ui.colored_label(change_color(c), format!("\u{25A0} {}", c.label()));
        }
    });
    let Some(overlay) = state.history.overlay_for(doc_id).cloned() else {
        ui.spinner();
        return;
    };
    if overlay.is_empty() {
        ui.weak("No differences: the map equals this snapshot.");
        return;
    }
    let mut jump: Option<LayerRef> = None;
    egui::Grid::new("history_diff")
        .num_columns(5)
        .striped(true)
        .show(ui, |ui| {
            ui.weak("Layer");
            for c in Change::ALL {
                ui.colored_label(change_color(c), c.sign());
            }
            ui.weak("");
            ui.end_row();
            for (&layer, counts) in &overlay.per_layer {
                let mut on = !state.history.overlay_hidden.contains(&layer);
                if ui
                    .checkbox(&mut on, layer_label(layer))
                    .on_hover_text("Show this layer's changes on the map")
                    .changed()
                {
                    if on {
                        state.history.overlay_hidden.remove(&layer);
                    } else {
                        state.history.overlay_hidden.insert(layer);
                    }
                }
                for c in Change::ALL {
                    ui.label(counts.get(c).to_string());
                }
                if ui
                    .small_button("Jump")
                    .on_hover_text("Centre the map on this layer's next change")
                    .clicked()
                {
                    jump = Some(layer);
                }
                ui.end_row();
            }
        });
    let (added, removed) = (
        overlay.objects_count(Change::Added),
        overlay.objects_count(Change::Removed),
    );
    if added + removed > 0 {
        ui.horizontal(|ui| {
            ui.label("Objects");
            ui.colored_label(change_color(Change::Added), format!("+{added}"));
            ui.colored_label(change_color(Change::Removed), format!("-{removed}"));
        });
    }
    for note in &overlay.notes {
        ui.weak(note);
    }
    if let Some(layer) = jump {
        let n = state.history.jump_cursor;
        if let Some((x, y)) = overlay.nth_of(layer, n) {
            state.history.jump_cursor = n + 1;
            state.jump_to_cell(x, y);
        }
    }
}

fn milestones(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Milestones").strong());
        ui.weak(r"git commits of capybara\saves\");
    });
    let log = state.milestones().clone();
    match log {
        Err(e) => {
            ui.colored_label(Color32::from_rgb(235, 110, 90), e);
        }
        Ok(list) if list.is_empty() => {
            ui.weak("None yet. Milestone... commits every saved file of every profile.");
        }
        Ok(list) => {
            egui::ScrollArea::vertical()
                .id_salt("milestone_log")
                .max_height(160.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Grid::new("milestone_grid")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for m in &list {
                                let first = m.message.lines().next().unwrap_or("").to_owned();
                                ui.label(first).on_hover_text(&m.message);
                                ui.label(local_time(
                                    m.time_unix.saturating_mul(1000),
                                    "%Y-%m-%d %H:%M",
                                ));
                                ui.monospace(m.id.chars().take(8).collect::<String>())
                                    .on_hover_text(&m.id);
                                ui.end_row();
                            }
                        });
                });
        }
    }
}

// ───────────────────────── prompts ─────────────────────────

/// What a prompt's buttons asked for.
enum Answer {
    Ok,
    Cancel,
}

fn text_prompt(
    ctx: &Context,
    id: &str,
    title: &str,
    hint: &str,
    text: &mut String,
    ok: &str,
) -> Option<Answer> {
    let mut answer = None;
    let response = egui::Modal::new(Id::new(id)).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.heading(title);
        ui.add_space(4.0);
        ui.weak(hint);
        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(380.0));
        if !edit.has_focus() && !edit.lost_focus() {
            edit.request_focus();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(ok).clicked() || enter {
                answer = Some(Answer::Ok);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
    });
    if answer.is_none() && response.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

fn confirm(ctx: &Context, id: &str, title: &str, lines: &[String], ok: &str) -> Option<Answer> {
    let mut answer = None;
    let response = egui::Modal::new(Id::new(id)).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading(title);
        ui.add_space(4.0);
        for l in lines {
            ui.label(l);
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(ok).clicked() {
                answer = Some(Answer::Ok);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(Answer::Cancel);
            }
        });
    });
    if answer.is_none() && response.should_close() {
        answer = Some(Answer::Cancel);
    }
    answer
}

fn describe(state: &AppState, id: SnapshotId) -> String {
    match state.history_list_entry(id) {
        Some(s) => {
            let when = local_time(s.created_unix_ms, "%Y-%m-%d %H:%M:%S");
            match display_name(&s) {
                Some(n) => format!("{} \"{n}\" ({when})", kind_label(&s.kind)),
                None => format!("{} ({when})", kind_label(&s.kind)),
            }
        }
        None => "the snapshot".to_owned(),
    }
}

/// The History panel's open prompt, if any.
pub fn dialogs(ctx: &Context, state: &mut AppState) {
    let Some(mut prompt) = state.history.prompt.clone() else {
        return;
    };
    let mut close = false;
    match &mut prompt {
        HistoryPrompt::Checkpoint { name } => {
            match text_prompt(
                ctx,
                "checkpoint_prompt",
                "New checkpoint",
                "Records the map as it is now in history under this name. Nothing is \
                 written to saves or CHANGED.txt.",
                name,
                "Record checkpoint",
            ) {
                Some(Answer::Ok) => {
                    let result = state
                        .active
                        .ok_or_else(|| "no map is open".to_owned())
                        .and_then(|i| state.checkpoint(i, name));
                    match result {
                        Ok(s) => {
                            state.notify(format!(
                                "Checkpoint \"{}\" recorded",
                                display_name(&s).unwrap_or("")
                            ));
                            close = true;
                        }
                        Err(e) => state.error(format!("Checkpoint: {e}")),
                    }
                }
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
        HistoryPrompt::Label { id, text } => {
            match text_prompt(
                ctx,
                "label_prompt",
                "Rename / label snapshot",
                "A label is a note shown in the timeline. Leave it empty to remove it.",
                text,
                "Set label",
            ) {
                Some(Answer::Ok) => match state.label_snapshot(*id, Some(text.clone())) {
                    Ok(()) => close = true,
                    Err(e) => state.error(format!("Label: {e}")),
                },
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
        HistoryPrompt::MakeCheckpoint { id, name } => {
            match text_prompt(
                ctx,
                "make_checkpoint_prompt",
                "Make checkpoint",
                "Checkpoints are never removed by the retention policy.",
                name,
                "Make checkpoint",
            ) {
                Some(Answer::Ok) => match state.make_checkpoint(*id, name) {
                    Ok(()) => close = true,
                    Err(e) => state.error(format!("Checkpoint: {e}")),
                },
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
        HistoryPrompt::Restore { id } => {
            let what = describe(state, *id);
            let lines = vec![
                format!("The map is replaced by {what}."),
                "The map as it is now is first recorded as a \"Before restore\" snapshot."
                    .to_owned(),
                "The restore is one step: Ctrl+Z undoes it. Nothing is saved until you save."
                    .to_owned(),
            ];
            match confirm(
                ctx,
                "restore_prompt",
                "Restore this snapshot?",
                &lines,
                "Restore",
            ) {
                Some(Answer::Ok) => {
                    match state.restore_snapshot(*id) {
                        Ok(_) => state.notify(format!("Restored {what} (Ctrl+Z undoes)")),
                        Err(e) => state.error(format!("Restore: {e}")),
                    }
                    close = true;
                }
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
        HistoryPrompt::Delete { id } => {
            let what = describe(state, *id);
            let lines = vec![
                format!("{what} is removed from the timeline."),
                "Its stored copy is freed by the next prune.".to_owned(),
            ];
            match confirm(
                ctx,
                "delete_prompt",
                "Delete this snapshot?",
                &lines,
                "Delete",
            ) {
                Some(Answer::Ok) => {
                    if let Err(e) = state.delete_snapshot(*id) {
                        state.error(format!("Delete: {e}"));
                    }
                    close = true;
                }
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
        HistoryPrompt::Milestone { message } => {
            match text_prompt(
                ctx,
                "milestone_prompt",
                "New milestone",
                "Commits every file in capybara\\saves\\ (all profiles) to its git \
                 repository. Save your maps first: unsaved changes are not included.",
                message,
                "Commit milestone",
            ) {
                Some(Answer::Ok) => match state.milestone(message) {
                    Ok(Some(id)) => {
                        state.notify(format!(
                            "Milestone {} committed",
                            id.chars().take(8).collect::<String>()
                        ));
                        close = true;
                    }
                    Ok(None) => {
                        state.notify("Milestone: nothing changed since the last one");
                        close = true;
                    }
                    Err(e) => state.error(format!("Milestone: {e}")),
                },
                Some(Answer::Cancel) => close = true,
                None => {}
            }
        }
    }
    if close {
        state.history.prompt = None;
    } else if state.history.prompt.is_some() {
        state.history.prompt = Some(prompt);
    }
}
