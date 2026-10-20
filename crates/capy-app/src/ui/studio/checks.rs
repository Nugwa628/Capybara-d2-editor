//! Object checks in the Objects window (ds1-studio `objectChecks.ts`, via
//! `capy_edit::check_objects`): objects outside the map, duplicates and,
//! once the object tables are loaded, ids the tables do not know. A click
//! on a finding selects the object.

use bevy_egui::egui::{self, Color32, RichText, Ui};
use capy_edit::{ObjectCheckOptions, ObjectProblem, check_objects};

use crate::state::AppState;

fn describe(p: ObjectProblem) -> String {
    match p {
        ObjectProblem::OutsideMap => "outside the map".to_owned(),
        ObjectProblem::Duplicate { of } => format!("duplicate of #{of}"),
        ObjectProblem::UnknownId => "id not in the object tables".to_owned(),
        ObjectProblem::UnpairedPad => "teleport pad without a partner".to_owned(),
    }
}

/// The findings of map `index` as `(object, text)`.
pub fn findings(state: &AppState, index: usize) -> Vec<(usize, String)> {
    let doc = &state.docs[index];
    let catalog = state.object_catalog();
    let act = doc.ds1.effective_act();
    let version = doc.ds1.version;
    let known = |o: &d2_formats::ds1::Object| {
        catalog.as_ref().is_none_or(|c| {
            let info = c.resolve(act, version, o);
            info.row.is_some() || info.token.is_some()
        })
    };
    let opts = ObjectCheckOptions {
        is_known: catalog
            .as_ref()
            .map(|_| &known as &dyn Fn(&d2_formats::ds1::Object) -> bool),
        teleport_pad_row: None,
    };
    check_objects(&doc.ds1, &opts)
        .into_iter()
        .map(|f| (f.index, describe(f.problem)))
        .collect()
}

/// The CHECKS block at the top of the Objects panel.
pub fn section(ui: &mut Ui, state: &mut AppState, index: usize) {
    let list = findings(state, index);
    let title = if list.is_empty() {
        "Checks: no problems found".to_owned()
    } else {
        format!("Checks: {} problem(s)", list.len())
    };
    let mut select = None;
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt("object_checks")
        .default_open(!list.is_empty())
        .show(ui, |ui| {
            if list.is_empty() {
                ui.weak("Outside the map, duplicates and unknown ids are checked.");
            }
            for (i, text) in list.iter().take(200) {
                let resp = ui.add(
                    egui::Label::new(
                        RichText::new(format!("#{i}: {text}"))
                            .color(Color32::from_rgb(235, 160, 60)),
                    )
                    .sense(egui::Sense::click()),
                );
                if resp.on_hover_text("Click to select the object").clicked() {
                    select = Some(*i);
                }
            }
        });
    if let Some(i) = select
        && let Some(doc) = state.docs.get_mut(index)
    {
        doc.object_sel = vec![i];
        state.objects.path.follow_object(Some(i));
    }
}
