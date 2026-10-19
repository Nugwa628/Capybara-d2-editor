//! Transient notifications in the bottom-right corner.

use bevy_egui::egui::{self, Align2, Color32, RichText};

use crate::state::{AppState, LogLevel};

const TOAST_SECONDS: f64 = 7.0;

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let now = ctx.input(|i| i.time);
    for t in &mut state.toasts {
        t.shown_at.get_or_insert(now);
    }
    state
        .toasts
        .retain(|t| t.shown_at.is_none_or(|s| now - s < TOAST_SECONDS));
    if state.toasts.is_empty() {
        return;
    }
    let mut dismiss: Option<usize> = None;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, [-12.0, -36.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for (i, toast) in state.toasts.iter().enumerate() {
                let color = match toast.level {
                    LogLevel::Error => Color32::from_rgb(190, 60, 60),
                    LogLevel::Warn => Color32::from_rgb(190, 150, 40),
                    LogLevel::Info => Color32::from_rgb(70, 110, 170),
                };
                egui::Frame::popup(ui.style())
                    .stroke(egui::Stroke::new(1.5, color))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&toast.text).color(Color32::WHITE));
                            if ui.small_button("\u{2715}").clicked() {
                                dismiss = Some(i);
                            }
                        });
                    });
            }
        });
    if let Some(i) = dismiss {
        state.toasts.remove(i);
    }
}
