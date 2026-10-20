//! Review of the open map's tile libraries: duplicate libraries, libraries
//! that clash on the tiles the map places, mixed colour versions, unused
//! tiles, and collecting the tiles the map uses into one custom DT1 of its
//! own (saved next to the map in the profile's saves).

use std::collections::BTreeSet;

use bevy_egui::egui::{self, Ui};
use capy_mod::tiles_review::{
    TileId, TilePick, build_custom_dt1, clashing_pairs, duplicate_dt1s, map_tile_usage,
    mixed_versions, own_tiles_path, plan_custom_dt1, tile_ids, unused_tile_indices,
};
use d2_formats::dt1::Dt1;

use super::io;
use crate::state::AppState;

#[derive(Default)]
pub struct ReviewUi {
    lines: Vec<String>,
    status: String,
    scanned: bool,
}

/// Parses the open map's libraries; (game path, model).
fn load_libs(state: &AppState) -> Result<Vec<(String, Dt1)>, String> {
    let info = state
        .active_doc()
        .and_then(|d| d.info.as_ref())
        .ok_or("The map's tile libraries are not known yet (the catalog is still loading)")?;
    let mut out = Vec::new();
    for p in &info.dt1_files {
        if let Some(bytes) = io::read_game(state, p)?
            && let Ok(dt1) = Dt1::parse(&bytes)
        {
            out.push((p.as_str().to_owned(), dt1));
        }
    }
    Ok(out)
}

/// The review as text lines (also used by scripts).
pub fn scan(state: &AppState) -> Result<Vec<String>, String> {
    let libs = load_libs(state)?;
    let doc = state.active_doc().ok_or("No map is open")?;
    let usage = map_tile_usage(doc.edit.ds1());
    let refs: Vec<(String, &Dt1)> = libs.iter().map(|(p, d)| (p.clone(), d)).collect();
    let mut lines = vec![format!(
        "{} librar{} loaded, {} distinct tile(s) placed",
        libs.len(),
        if libs.len() == 1 { "y" } else { "ies" },
        usage.len()
    )];
    let dups = duplicate_dt1s(&refs);
    if dups.is_empty() {
        lines.push("No duplicate libraries.".into());
    }
    for d in &dups {
        lines.push(format!(
            "Duplicate: {} repeats {} tile(s) of {}",
            d.later,
            d.shared.len(),
            d.earlier
        ));
    }
    for ((a, b), n) in clashing_pairs(&refs, &usage) {
        lines.push(format!(
            "Clash: {a} and {b} both provide {n} placed tile(s)"
        ));
    }
    for m in mixed_versions(&refs) {
        lines.push(format!(
            "Mixed versions in {}: tile {}/{}/{} has {} recoloured copies",
            m.path,
            m.id.orientation,
            m.id.main,
            m.id.sub,
            m.indices.len()
        ));
    }
    let used: BTreeSet<TileId> = usage.keys().copied().collect();
    for (p, d) in &libs {
        let unused = unused_tile_indices(d, &used).len();
        lines.push(format!(
            "{p}: {} tiles, {unused} not placed by this map",
            d.tiles.len()
        ));
    }
    let provided: BTreeSet<TileId> = libs.iter().flat_map(|(_, d)| tile_ids(d)).collect();
    let missing: Vec<&TileId> = used.iter().filter(|t| !provided.contains(t)).collect();
    if !missing.is_empty() {
        lines.push(format!(
            "{} placed tile(s) are in none of the loaded libraries",
            missing.len()
        ));
    }
    Ok(lines)
}

/// Collects the used tiles into the map's own DT1 and saves it.
pub fn build_own(state: &mut AppState) -> Result<String, String> {
    let libs = load_libs(state)?;
    let (usage, map_path) = {
        let doc = state.active_doc().ok_or("No map is open")?;
        (map_tile_usage(doc.edit.ds1()), doc.path.as_str().to_owned())
    };
    let sources: Vec<(String, &Dt1)> = libs.iter().map(|(p, d)| (p.clone(), d)).collect();
    let mut picks = Vec::new();
    for id in usage.keys() {
        'find: for (s, (_, d)) in libs.iter().enumerate() {
            for (i, t) in d.tiles.iter().enumerate() {
                if TileId::of(t) == *id {
                    picks.push(TilePick {
                        source: s,
                        index: i,
                    });
                    break 'find;
                }
            }
        }
    }
    if picks.is_empty() {
        return Err("The map places no tile found in its libraries".into());
    }
    let plan = plan_custom_dt1(&picks, &sources, &BTreeSet::new());
    let dt1 = build_custom_dt1(&plan, None);
    let rel = own_tiles_path(&io::tiles_rel(&map_path), None, &[]);
    let path = io::tiles_game(&rel);
    let bytes = dt1
        .to_bytes()
        .map_err(|e| format!("cannot write the DT1: {e}"))?;
    io::save_game(state, &path, &bytes)?;
    Ok(format!(
        "Saved {} tile(s) to {path} ({} renumbered, {} skipped)",
        dt1.tiles.len(),
        plan.renumbered.len(),
        plan.skipped.len()
    ))
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    let mut r = std::mem::take(&mut state.tools.review);
    ui.horizontal(|ui| {
        if ui.button("Scan the open map's libraries").clicked() || !r.scanned {
            r.scanned = true;
            match scan(state) {
                Ok(lines) => {
                    r.lines = lines;
                    r.status.clear();
                }
                Err(e) => r.status = e,
            }
        }
        if ui
            .button("Collect the placed tiles into an own DT1")
            .on_hover_text("Copies every tile the map places into <map>_custom.dt1 in the saves")
            .clicked()
        {
            r.status = match build_own(state) {
                Ok(m) | Err(m) => m,
            };
        }
    });
    super::message(ui, &r.status);
    ui.separator();
    for l in &r.lines {
        ui.add(egui::Label::new(l.as_str()).wrap());
    }
    state.tools.review = r;
}
