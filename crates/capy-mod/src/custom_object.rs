//! Custom objects: a picture of your own placed in maps like any object.
//!
//! Which `objects.txt` row a DS1 object id means is fixed by a table in the
//! game's program files (150 ids per act), so a new object cannot get a new id:
//! it takes over a row one of those ids points to. Only rows nothing else uses
//! count as free: no map places them, no ObjGroup group spawns them, and they
//! have no game functions (OperateFn / InitFn / PopulateFn / ClientFn).
//!
//! The row becomes a plain decoration like the game's "banner 1": one mode (NU),
//! optionally animated, with optional light and collision. Its graphics are a
//! COF and one DC6 layer (TR) under `data/global/objects/<token>/`.
//!
//! This module is the table side plus the file plan. The DC6 is encoded here
//! (`Dc6::from_frames`); `d2-formats` has no COF writer yet, so the COF is
//! returned as a [`CofPlan`] for the caller to encode.

use std::collections::{HashMap, HashSet};

use d2_formats::IndexedImage;
use d2_formats::dc6::Dc6;
use d2_formats::txt::TxtDoc;

use crate::lvl_error::{Result, refuse};
use crate::txt_util::{cell, cell_num, data_rows, has, set};

/// Object ids per act in the game's preset table.
pub const OBJECTS_PER_ACT: usize = 150;
/// The description given to rows made here, so they can be found again.
pub const CUSTOM_MARK: &str = "Capybara custom: ";
/// The mark the reference editor used; its rows are recognised too.
pub const LEGACY_MARK: &str = "DS1 Studio custom: ";
const GAME_FUNCTIONS: [&str; 4] = ["OperateFn", "InitFn", "PopulateFn", "ClientFn"];
const DESC: &str = "description - not loaded";
const OBJECTS_DIR: &str = "data/global/objects/";

/// `objects.txt` data-line indices in the game's row numbering.
pub fn game_rows(objects: &TxtDoc) -> Vec<usize> {
    data_rows(objects)
}

/// The `objects.txt` row (game numbering) a DS1 object id resolves to in an act
/// (0-based). An id of 150 or more is the row `id - 150` directly; a lower one
/// goes through the act's table, negative ids reaching back into earlier acts'.
/// `presets[act][id]` is the act's table (see `d2_assets::objects`).
pub fn preset_row(presets: &[Vec<i32>], act: usize, id: i32) -> i32 {
    if id >= OBJECTS_PER_ACT as i32 {
        return id - OBJECTS_PER_ACT as i32;
    }
    let (mut act, mut n) = (act as i64, i64::from(id));
    while n < 0 {
        act -= 1;
        n += OBJECTS_PER_ACT as i64;
    }
    usize::try_from(act)
        .ok()
        .and_then(|a| presets.get(a))
        .and_then(|p| p.get(n as usize))
        .copied()
        .unwrap_or(-1)
}

/// `objects.txt` rows ObjGroup.txt groups spawn (its ID0..ID7 columns).
pub fn obj_group_rows(groups: Option<&TxtDoc>) -> HashSet<usize> {
    let mut out = HashSet::new();
    let Some(doc) = groups else { return out };
    for r in 0..doc.row_count() {
        for i in 0..8 {
            let v = cell_num(doc, r, &format!("ID{i}"));
            if v > 0 {
                out.insert(v as usize);
            }
        }
    }
    out
}

/// A DS1 object id of an act with the row it points to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectSlot {
    /// DS1 object id in the act.
    pub id: usize,
    /// objects.txt row (game numbering).
    pub row: usize,
    pub name: String,
    pub token: String,
    /// Objects placed with this row in the maps.
    pub uses: u32,
    /// An ObjGroup.txt group spawns it.
    pub in_group: bool,
    /// It has a game function (the game spawns or runs it in code).
    pub functions: bool,
    /// Made with this tool before.
    pub custom: bool,
    /// Nothing uses it: safe to take over.
    pub free: bool,
}

/// The object ids of an act (0-based) with the row each points to, and whether
/// that row is free to take over. `uses`: placed count per objects.txt row.
pub fn object_slots(
    presets: &[Vec<i32>],
    objects: &TxtDoc,
    act: usize,
    uses: &HashMap<usize, u32>,
    groups: &HashSet<usize>,
) -> Vec<ObjectSlot> {
    let rows = game_rows(objects);
    let mut out = Vec::new();
    for id in 0..OBJECTS_PER_ACT {
        let row = presets
            .get(act)
            .and_then(|p| p.get(id))
            .copied()
            .unwrap_or(-1);
        if row <= 0 || row as usize >= rows.len() {
            continue;
        }
        let row = row as usize;
        let line = rows[row];
        let desc = cell(objects, line, DESC);
        let mark = [CUSTOM_MARK, LEGACY_MARK]
            .into_iter()
            .find(|m| desc.starts_with(m));
        let functions = GAME_FUNCTIONS
            .iter()
            .any(|c| cell_num(objects, line, c) > 0);
        let n = uses.get(&row).copied().unwrap_or(0);
        let in_group = groups.contains(&row);
        let custom = mark.is_some();
        let name = match mark {
            Some(m) => desc[m.len()..].to_owned(),
            None if !desc.is_empty() => desc.clone(),
            None => {
                let n = cell(objects, line, "Name");
                if n.is_empty() {
                    format!("row {row}")
                } else {
                    n
                }
            }
        };
        out.push(ObjectSlot {
            id,
            row,
            name,
            token: cell(objects, line, "Token"),
            uses: n,
            in_group,
            functions,
            custom,
            free: !in_group && !functions && (custom || n == 0),
        });
    }
    out
}

/// A two-character token no objects.txt row uses and none of `folders` (the
/// names under `data/global/objects/`, any case) has.
pub fn free_token(objects: &TxtDoc, folders: &HashSet<String>) -> Result<String> {
    let used: HashSet<String> = (0..objects.row_count())
        .map(|r| cell(objects, r, "Token").to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let chars: Vec<char> = "zyxwvutsrqponmlkjihgfedcba0123456789".chars().collect();
    for a in &chars {
        for b in &chars {
            let t: String = [*a, *b].iter().collect();
            if !used.contains(&t) && !folders.contains(&t) {
                return Ok(t);
            }
        }
    }
    refuse("No free two-character object token left.")
}

/// Settings of a custom object.
#[derive(Clone, Debug)]
pub struct CustomObjectOptions {
    pub name: String,
    /// Two characters (see [`free_token`]).
    pub token: String,
    pub frames: u32,
    /// Frames per game frame x 256 (256 = 25 per second).
    pub speed: u32,
    /// Light radius (0 = none) and colour.
    pub light: u32,
    pub light_colour: [u8; 3],
    pub flicker: bool,
    /// Blocks walking, over `size` sub-tiles each way.
    pub blocks: bool,
    pub size: u32,
    /// Drawn under units (rugs, decals).
    pub draw_under: bool,
    /// Act (0-4) whose objects the row appears in.
    pub act: u8,
}

/// The edited objects table.
#[derive(Clone, Debug)]
pub struct ObjectRowOutcome {
    pub objects: TxtDoc,
    pub changes: Vec<String>,
}

/// Sets the objects.txt row (game numbering) up as a custom decoration. Columns
/// the table doesn't have are skipped; extra columns are left as they are.
/// Running it again with the same options changes nothing.
pub fn custom_object_row(
    objects: &TxtDoc,
    row: usize,
    o: &CustomObjectOptions,
) -> Result<ObjectRowOutcome> {
    let Some(&line) = game_rows(objects).get(row) else {
        return refuse(format!("objects.txt has no row {row}."));
    };
    if o.token.chars().count() != 2 || !o.token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return refuse("An object token is two letters or digits.");
    }
    if o.frames == 0 {
        return refuse("An object needs at least one frame.");
    }
    let light_on = o.light > 0;
    let b = |x: bool| u32::from(x).to_string();
    let values: Vec<(String, String)> = vec![
        ("Name".into(), o.name.clone()),
        (DESC.into(), format!("{CUSTOM_MARK}{}", o.name)),
        ("Token".into(), o.token.clone()),
        ("SpawnMax".into(), "0".into()),
        ("TrapProb".into(), "0".into()),
        (
            "SizeX".into(),
            if o.blocks { o.size } else { 0 }.to_string(),
        ),
        (
            "SizeY".into(),
            if o.blocks { o.size } else { 0 }.to_string(),
        ),
        ("FrameCnt0".into(), o.frames.to_string()),
        ("FrameDelta0".into(), o.speed.to_string()),
        ("CycleAnim0".into(), b(o.frames > 1)),
        ("Lit0".into(), o.light.to_string()),
        ("HasCollision0".into(), b(o.blocks)),
        ("Mode0".into(), "1".into()),
        ("Orientation".into(), "1".into()),
        ("Draw".into(), "1".into()),
        ("Red".into(), o.light_colour[0].to_string()),
        ("Green".into(), o.light_colour[1].to_string()),
        ("Blue".into(), o.light_colour[2].to_string()),
        ("TR".into(), "1".into()),
        ("TotalPieces".into(), "1".into()),
        ("NameOffset".into(), "-80".into()),
        ("OperateRange".into(), "2".into()),
        ("Restore".into(), "1".into()),
        ("Act".into(), (1u32 << o.act).to_string()),
        ("Flicker".into(), b(light_on && o.flicker)),
        ("Damage".into(), "100".into()),
        ("DrawUnder".into(), b(o.draw_under)),
    ];
    // Everything else a vanilla column: off.
    let mut zero: Vec<String> = Vec::new();
    for m in 0..8 {
        for p in [
            "Selectable",
            "FrameCnt",
            "FrameDelta",
            "CycleAnim",
            "Lit",
            "BlocksLight",
            "HasCollision",
            "Start",
            "OrderFlag",
            "Mode",
            "Parm",
        ] {
            zero.push(format!("{p}{m}"));
        }
    }
    zero.extend(
        [
            "nTgtFX",
            "nTgtFY",
            "nTgtBX",
            "nTgtBY",
            "IsAttackable0",
            "EnvEffect",
            "IsDoor",
            "BlocksVis",
            "Trans",
            "PreOperate",
            "Yoffset",
            "Xoffset",
            "HD",
            "LG",
            "RA",
            "LA",
            "RH",
            "LH",
            "SH",
            "S1",
            "S2",
            "S3",
            "S4",
            "S5",
            "S6",
            "S7",
            "S8",
            "SubClass",
            "Xspace",
            "Yspace",
            "MonsterOK",
            "ShrineFunction",
            "Lockable",
            "Gore",
            "Sync",
            "Beta",
            "Overlay",
            "CollisionSubst",
            "Left",
            "Top",
            "Width",
            "Height",
            "OperateFn",
            "PopulateFn",
            "InitFn",
            "ClientFn",
            "RestoreVirgins",
            "BlockMissile",
            "OpenWarp",
            "AutoMap",
        ]
        .map(String::from),
    );
    let mut doc = objects.clone();
    let mut changes = Vec::new();
    let mut apply = |doc: &mut TxtDoc, col: &str, v: &str| -> Result<()> {
        let from = cell(doc, line, col);
        if has(doc, col) && set(doc, line, col, v)? {
            changes.push(format!(
                "objects.txt row {row}: {col} {} -> {v}",
                if from.is_empty() { "(empty)" } else { &from }
            ));
        }
        Ok(())
    };
    for c in &zero {
        if !values.iter().any(|(k, _)| k.eq_ignore_ascii_case(c)) {
            apply(&mut doc, c, "0")?;
        }
    }
    for (c, v) in &values {
        apply(&mut doc, c, v)?;
    }
    Ok(ObjectRowOutcome {
        objects: doc,
        changes,
    })
}

/// A picture cut into `count` frames side by side (a strip), each the same
/// width. Palette indices, 0 = transparent.
pub fn split_strip(
    width: u32,
    height: u32,
    pixels: &[u8],
    count: u32,
) -> Result<Vec<IndexedImage>> {
    let w = width.checked_div(count).unwrap_or(0);
    if w < 1 || w * count != width || pixels.len() != (width * height) as usize {
        return refuse(format!(
            "The picture is {width} pixels wide: {count} frames side by side need a width that divides by {count}."
        ));
    }
    Ok((0..count)
        .map(|k| {
            let mut f = IndexedImage::new(w, height, 0, 0);
            for y in 0..height {
                for x in 0..w {
                    let p = pixels[(y * width + k * w + x) as usize];
                    f.pixels[(y * w + x) as usize] = p;
                    f.alpha[(y * w + x) as usize] = if p == 0 { 0 } else { 255 };
                }
            }
            f
        })
        .collect())
}

/// How many frames a strip probably holds: the fewest equal frames (2-64) whose
/// edges all fall on see-through columns, with something drawn in each; square
/// frames first when the width allows them. 1 when nothing fits.
pub fn guess_frames(width: u32, height: u32, solid: impl Fn(u32, u32) -> bool) -> u32 {
    let clear = |x: u32| (0..height).all(|y| !solid(x, y));
    let fits = |n: u32| {
        if !width.is_multiple_of(n) {
            return false;
        }
        let w = width / n;
        (1..n).all(|k| clear(k * w - 1) && clear(k * w))
            && (0..n).all(|k| (k * w..(k + 1) * w).any(|x| !clear(x)))
    };
    if height > 0
        && width > height
        && width.is_multiple_of(height)
        && width / height <= 64
        && fits(width / height)
    {
        return width / height;
    }
    (2..=64).find(|&n| fits(n)).unwrap_or(1)
}

/// One layer of the planned COF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CofLayerPlan {
    /// Component index (1 = TR).
    pub component: u8,
    pub shadow: bool,
    pub selectable: bool,
    pub transparent: bool,
    pub draw_effect: u8,
    pub weapon_class: String,
}

/// The COF to write (`d2-formats` has no COF encoder yet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CofPlan {
    pub path: String,
    pub directions: u8,
    pub frames_per_direction: u8,
    pub animation_rate: u16,
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: i32,
    pub y_max: i32,
    pub layers: Vec<CofLayerPlan>,
}

/// The files a custom object needs.
#[derive(Clone, Debug)]
pub struct ObjectFilePlan {
    pub cof: CofPlan,
    pub dc6_path: String,
    /// The encoded DC6 (one direction, one frame per picture frame).
    pub dc6: Vec<u8>,
}

/// The object's graphics files: its COF (one TR layer, one direction) and the
/// DC6 holding the frames. The object's spot is `anchor_x` pixels from the
/// picture's left edge (its centre unless given) and `feet` pixels above its
/// bottom edge.
pub fn custom_object_files(
    token: &str,
    frames: &[IndexedImage],
    feet: i32,
    anchor_x: Option<i32>,
) -> Result<ObjectFilePlan> {
    let Some(first) = frames.first() else {
        return refuse("An object needs at least one frame.");
    };
    if frames.len() > 255
        || frames
            .iter()
            .any(|f| f.width != first.width || f.height != first.height)
    {
        return refuse("Frames must be the same size, at most 255 of them.");
    }
    let t = token.to_lowercase();
    let (w, h) = (first.width as i32, first.height as i32);
    let left = -anchor_x.unwrap_or(w / 2);
    let cof = CofPlan {
        path: format!("{OBJECTS_DIR}{t}/cof/{t}nuhth.cof"),
        directions: 1,
        frames_per_direction: frames.len() as u8,
        animation_rate: 256,
        x_min: left,
        x_max: left + w,
        y_min: feet - h,
        y_max: feet,
        layers: vec![CofLayerPlan {
            component: 1,
            shadow: true,
            selectable: false,
            transparent: false,
            draw_effect: 0,
            weapon_class: "hth".into(),
        }],
    };
    let placed: Vec<IndexedImage> = frames
        .iter()
        .map(|f| {
            let mut f = f.clone();
            f.offset_x = left;
            f.offset_y = feet;
            f
        })
        .collect();
    let dc6 = Dc6::from_frames(&[placed])?.to_bytes()?;
    Ok(ObjectFilePlan {
        cof,
        dc6_path: format!("{OBJECTS_DIR}{t}/tr/{t}trlitnuhth.dc6"),
        dc6,
    })
}
