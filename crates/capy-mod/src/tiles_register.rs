//! Making the game load a DT1: registering it in LvlTypes and LvlPrest
//! (`register_dt1`) and "Add to this map" (`add_tiles_to_map`), which copies
//! picked tiles into the level type's own DT1 and registers that.
//! Pure: tables, DT1s and the DS1 come in as values and go out edited.
//! Behaviour reference: ds1-studio `ownTiles.ts`, `syncLevelTables`.

use std::collections::{BTreeMap, BTreeSet};

use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::txt::TxtDoc;

use crate::compat::Edit;
use crate::level_tables::{ensure_type_slots, preset_rows_for, slot_key};
use crate::lvl_error::{Result, refuse};
use crate::tiles_review::{
    CustomDt1Plan, TileId, TilePick, append_tiles, build_custom_dt1, own_tiles_path,
    plan_custom_dt1, tile_ids,
};
use crate::txt_util::{cell, cell_num, mask, rel_key, row_of_record, set, tiles_rel};

/// What [`register_dt1`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registration {
    /// The File slot (1-32) that holds the DT1.
    pub slot: usize,
    /// The slot was newly filled (false: the DT1 was already listed).
    pub slot_added: bool,
    /// LvlPrest rows whose Dt1Mask now has the slot's bit.
    pub mask_rows: Vec<usize>,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Makes the game load `dt1_path`: puts it in the first free File slot of
/// LvlTypes level type `type_id` (or reuses its slot) and turns the slot's
/// bit on in the Dt1Mask of each of `prest_rows`. Bits are only ever turned
/// on, so a LvlPrest row shared with other maps keeps what they load. With no
/// LvlPrest row only LvlTypes changes and a warning says so. Refuses (both
/// tables untouched) when the type has no row or all 32 slots are used.
pub fn register_dt1(
    types: &mut TxtDoc,
    prest: &mut TxtDoc,
    type_id: u32,
    prest_rows: &[usize],
    dt1_path: &str,
) -> Result<Registration> {
    let Some(type_row) = row_of_record(types, type_id as usize) else {
        return refuse(format!("LvlTypes.txt has no level type {type_id}"));
    };
    let mut t = types.clone();
    let mut p = prest.clone();
    let key = rel_key(dt1_path);
    let tname = cell(&t, type_row, "Name");
    let mut out = Registration::default();
    let found = (1..=32).find(|&i| slot_key(&t, type_row, i).as_deref() == Some(key.as_str()));
    out.slot = match found {
        Some(s) => s,
        None => {
            let s = ensure_type_slots(&mut t, type_row, &[dt1_path.to_owned()])?;
            let slot = s.slots.get(&key).copied().unwrap_or(0);
            if let Some(a) = s.added.first() {
                out.slot_added = true;
                out.changes.push(format!(
                    "LvlTypes \"{tname}\" (type {type_id}): File {} = {}",
                    a.slot, a.value
                ));
            }
            slot
        }
    };
    if out.slot == 0 {
        return refuse(format!("LvlTypes \"{tname}\" has no free File slot"));
    }
    let bit = 1u32 << (out.slot - 1);
    if prest_rows.is_empty() {
        out.warnings.push(
            "No LvlPrest row lists this map, so there is no Dt1Mask to update and the game does not load the map yet (use Game > Add to game)"
                .to_owned(),
        );
    }
    for &r in prest_rows {
        let old = mask(&cell(&p, r, "Dt1Mask"));
        if old & bit == 0 {
            let new = old | bit;
            if set(&mut p, r, "Dt1Mask", &new.to_string())? {
                out.mask_rows.push(r);
                out.changes.push(format!(
                    "LvlPrest \"{}\": Dt1Mask {old} -> {new}",
                    cell(&p, r, "Name")
                ));
            }
        }
    }
    *types = t;
    *prest = p;
    Ok(out)
}

/// The LvlPrest rows of a map: the row with Def `def` when known, else every
/// row that lists the map.
pub fn map_prest_rows(prest: &TxtDoc, map: &str, def: Option<u32>) -> Vec<usize> {
    if let Some(d) = def
        && let Some(r) = row_of_record(prest, d as usize)
        && cell_num(prest, r, "Def") == i64::from(d)
    {
        return vec![r];
    }
    preset_rows_for(prest, map)
}

/// The DT1s a level type lists (File 1-32), relative to data/global/tiles.
pub fn type_files(types: &TxtDoc, type_id: u32) -> Vec<String> {
    let Some(row) = row_of_record(types, type_id as usize) else {
        return Vec::new();
    };
    (1..=32)
        .filter_map(|i| {
            let v = cell(types, row, &format!("File {i}"));
            let t = v.trim();
            (!t.is_empty() && t != "0").then(|| tiles_rel(t))
        })
        .collect()
}

/// The level type's own-tiles DT1 (relative to data/global/tiles): one per
/// level type, in the folder most of its libraries are in; a name the type
/// already lists keeps its spelling.
pub fn own_path_for(types: &TxtDoc, type_id: u32, map_path: &str) -> String {
    let listed = type_files(types, type_id);
    let name = row_of_record(types, type_id as usize)
        .map(|r| cell(types, r, "Name"))
        .unwrap_or_default();
    own_tiles_path(&tiles_rel(map_path), Some((&name, &listed)), &listed)
}

/// Inputs of [`add_tiles_to_map`].
pub struct AddTilesInput<'a> {
    /// The map's game path (for the fallback own-DT1 name and LvlPrest rows).
    pub map_path: &'a str,
    pub ds1: &'a Ds1,
    pub types: &'a TxtDoc,
    pub prest: &'a TxtDoc,
    pub type_id: u32,
    /// The map's LvlPrest Def, when known.
    pub prest_def: Option<u32>,
    /// Picked tiles: `source` indexes `sources`.
    pub picks: &'a [TilePick],
    pub sources: &'a [(String, &'a Dt1)],
    /// Every DT1 of the level type (all File slots, not only the ones the map
    /// selects): their tile numbers are all avoided.
    pub type_dt1s: &'a [(String, &'a Dt1)],
    /// The type's own-tiles DT1 as it exists now, and its path when known.
    pub existing_own: Option<&'a Dt1>,
    pub existing_own_path: Option<&'a str>,
}

/// The result of [`add_tiles_to_map`].
#[derive(Clone, Debug)]
pub struct AddTilesOutcome {
    /// The own-tiles DT1's path relative to data/global/tiles.
    pub own_path: String,
    /// The own-tiles DT1 with the new tiles appended.
    pub dt1: Dt1,
    pub added: usize,
    pub plan: CustomDt1Plan,
    /// Old number -> new number, for tiles that were renumbered.
    pub remap: BTreeMap<TileId, TileId>,
    /// Where each picked tile's number ended up (renumbered or not).
    pub placed: BTreeMap<TileId, TileId>,
    pub types: TxtDoc,
    pub prest: TxtDoc,
    pub ds1: Ds1,
    pub registration: Registration,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Copies the picked tiles into the level type's own DT1 (created, or
/// appended to), registers it in LvlTypes and LvlPrest and lists it in the
/// DS1. Numbers clashing with any DT1 of the level type or the own DT1 are
/// renumbered (walls keep their orientation, animation frames and corner-wall
/// halves stay together).
pub fn add_tiles_to_map(inp: &AddTilesInput<'_>) -> Result<AddTilesOutcome> {
    if row_of_record(inp.types, inp.type_id as usize).is_none() {
        return refuse(format!("LvlTypes.txt has no level type {}", inp.type_id));
    }
    let own_path = match inp.existing_own_path {
        Some(p) => tiles_rel(p),
        None => own_path_for(inp.types, inp.type_id, inp.map_path),
    };
    let mut taken: BTreeSet<TileId> = BTreeSet::new();
    for (_, d) in inp.type_dt1s {
        taken.extend(tile_ids(d));
    }
    if let Some(o) = inp.existing_own {
        taken.extend(tile_ids(o));
    }
    let plan = plan_custom_dt1(inp.picks, inp.sources, &taken);
    let mut warnings: Vec<String> = plan
        .skipped
        .iter()
        .map(|s| format!("Skipped {s}"))
        .collect();
    if plan.records.is_empty() {
        let why = if plan.skipped.is_empty() {
            "nothing was picked".to_owned()
        } else {
            plan.skipped.join("; ")
        };
        return refuse(format!("No tile can be added: {why}"));
    }
    let mut types = inp.types.clone();
    let mut prest = inp.prest.clone();
    let rows = map_prest_rows(&prest, &tiles_rel(inp.map_path), inp.prest_def);
    let registration = register_dt1(&mut types, &mut prest, inp.type_id, &rows, &own_path)?;
    warnings.extend(registration.warnings.iter().cloned());

    let new = build_custom_dt1(&plan, None);
    let (dt1, first) = append_tiles(inp.existing_own, new.tiles);
    let added = dt1.tiles.len() - first;
    let mut remap = BTreeMap::new();
    let mut placed = BTreeMap::new();
    for t in &plan.tiles {
        let to = TileId::new(t.id.orientation, t.new_main, t.new_sub);
        placed.insert(t.id, to);
        if to != t.id {
            remap.insert(t.id, to);
        }
    }
    let mut ds1 = inp.ds1.clone();
    Edit::AddDt1s {
        paths: vec![own_path.clone()],
    }
    .apply_ds1(&mut ds1);

    let mut changes = vec![format!(
        "{added} tile(s) {} {own_path} ({} total)",
        if inp.existing_own.is_some() {
            "appended to"
        } else {
            "copied into the new"
        },
        dt1.tiles.len()
    )];
    for r in &plan.renumbered {
        changes.push(format!("Renumbered {r}"));
    }
    changes.extend(registration.changes.iter().cloned());
    changes.push("The map's own DT1 list names the file".to_owned());
    Ok(AddTilesOutcome {
        own_path,
        dt1,
        added,
        plan,
        remap,
        placed,
        types,
        prest,
        ds1,
        registration,
        changes,
        warnings,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn types(files: usize) -> TxtDoc {
        let mut h = vec!["Name".to_owned(), "Id".to_owned()];
        h.extend((1..=32).map(|i| format!("File {i}")));
        let mut row = vec!["Town".to_owned(), "0".to_owned()];
        row.extend((1..=32).map(|i| {
            if i <= files {
                format!("a/f{i}.dt1")
            } else {
                "0".to_owned()
            }
        }));
        let text = format!("{}\r\n{}\r\n", h.join("\t"), row.join("\t"));
        TxtDoc::parse(text.as_bytes())
    }

    fn prest() -> TxtDoc {
        TxtDoc::parse(b"Name\tDef\tLevelId\tDt1Mask\tFile1\r\nTown\t0\t1\t3\tact1/town/x.ds1\r\n")
    }

    #[test]
    fn registers_in_the_first_free_slot_and_the_mask() {
        let (mut t, mut p) = (types(2), prest());
        let r = register_dt1(&mut t, &mut p, 0, &[0], "b/own.dt1").unwrap();
        assert_eq!((r.slot, r.slot_added), (3, true));
        assert_eq!(cell(&p, 0, "Dt1Mask"), "7");
        assert_eq!(cell(&t, 0, "File 3"), "b/own.dt1");
        let (t2, p2) = (t.clone(), p.clone());
        let r = register_dt1(&mut t, &mut p, 0, &[0], "B/OWN.dt1").unwrap();
        assert!(!r.slot_added && r.changes.is_empty());
        assert_eq!((t, p), (t2, p2));
    }

    #[test]
    fn full_type_is_refused_untouched() {
        let (mut t, mut p) = (types(32), prest());
        let (t0, p0) = (t.clone(), p.clone());
        assert!(register_dt1(&mut t, &mut p, 0, &[0], "b/own.dt1").is_err());
        assert_eq!((t.clone(), p.clone()), (t0, p0));
        // a DT1 already in a slot still gets its bit
        let r = register_dt1(&mut t, &mut p, 0, &[0], "a/f3.dt1").unwrap();
        assert_eq!(r.slot, 3);
    }

    #[test]
    fn no_preset_row_warns_but_edits_lvltypes() {
        let (mut t, mut p) = (types(1), prest());
        let r = register_dt1(&mut t, &mut p, 0, &[], "b/own.dt1").unwrap();
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(cell(&t, 0, "File 2"), "b/own.dt1");
        assert_eq!(cell(&p, 0, "Dt1Mask"), "3");
    }
}
