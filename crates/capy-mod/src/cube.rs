//! Cube recipes: a new item (a copy of a template item appended to Misc.txt)
//! and a Horadric Cube recipe that makes it, plus the lookup of the recipes
//! that make a map's item.
//!
//! * Items are numbered by position across Weapons, Armor and Misc, so a new
//!   item is always appended at the end of Misc.txt.
//! * The cube uses the first enabled recipe whose inputs match, so a recipe with
//!   the same inputs as an existing one never runs and is refused.
//! * Elixirs are a beta item; a copy crashed the client while drawing its
//!   tooltip, so they are not offered as templates.
//! * Items of a type the base game doesn't have (PD2's maps...) are handled by
//!   the mod's own code, which only knows its own item codes.

use d2_formats::tbl::TblDoc;
use d2_formats::txt::TxtDoc;

use crate::lvl_error::{Result, refuse};
use crate::txt_util::{append_values, cell, data_rows, has, num, row_values, set};

/// Tag in the comment columns of rows this tool adds, so they can be listed and removed.
pub const CAPY_TAG: &str = "(Capybara)";
/// The tag the reference editor used; rows with it are listed too.
pub const DS1_STUDIO_TAG: &str = "(DS1 Studio)";

/// The base game's item types (ItemTypes.txt `Code`).
pub const VANILLA_ITEM_TYPES: &[&str] = &[
    "shie", "tors", "gold", "bowq", "xboq", "play", "herb", "poti", "ring", "elix", "amul", "char",
    "boot", "glov", "book", "belt", "gem", "torc", "scro", "scep", "wand", "staf", "bow", "axe",
    "club", "swor", "hamm", "knif", "spea", "pole", "xbow", "mace", "helm", "tpot", "ques", "body",
    "key", "tkni", "taxe", "jave", "weap", "mele", "miss", "thro", "comb", "armo", "shld", "misc",
    "sock", "seco", "rod", "misl", "blun", "jewl", "clas", "amaz", "barb", "necr", "pala", "sorc",
    "assn", "drui", "h2h", "orb", "head", "ashd", "phlm", "pelt", "cloa", "rune", "circ", "hpot",
    "mpot", "rpot", "spot", "apot", "wpot", "scha", "mcha", "lcha", "abow", "aspe", "ajav", "h2h2",
    "mboq", "mxbq", "gem0", "gem1", "gem2", "gem3", "gem4", "gema", "gemd", "geme", "gemr", "gems",
    "gemt", "gemz",
];

fn is_vanilla_type(t: &str) -> bool {
    VANILLA_ITEM_TYPES.contains(&t.to_lowercase().as_str())
}

/// How risky a template item is to copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateRisk {
    Blocked(String),
    ModType(String),
}

/// Whether copying an item of these types is refused or needs confirmation.
pub fn template_risk(type1: &str, type2: &str) -> Option<TemplateRisk> {
    let types: Vec<String> = [type1, type2]
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if types.iter().any(|t| t == "elix") {
        return Some(TemplateRisk::Blocked("Elixirs are a beta item the retail game never hands out. A copy of one crashed the game when hovered (D2Client access violation while drawing its tooltip). Pick another template.".into()));
    }
    let modded: Vec<&String> = types.iter().filter(|t| !is_vanilla_type(t)).collect();
    (!modded.is_empty()).then(|| {
        TemplateRisk::ModType(format!(
            "Its item type ({}) isn't one of the base game's: your mod's own code handles it. That code only knows the mod's own item codes, so a copy under a new code may do nothing, or crash the game when hovered or used. Only go ahead if your mod supports new items of this type.",
            modded.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ))
    })
}

/// A cube input as the game compares it: lower case, no quotes or spaces, parameters sorted.
pub fn normalize_input(s: &str) -> String {
    let mut parts = s
        .replace('"', "")
        .split(',')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return String::new();
    }
    let code = parts.remove(0);
    parts.sort();
    std::iter::once(code)
        .chain(parts)
        .collect::<Vec<_>>()
        .join(",")
}

/// The `input 1`..`input 7` columns the table has.
fn input_cols(cube: &TxtDoc) -> Vec<String> {
    (1..=7)
        .map(|i| format!("input {i}"))
        .filter(|c| has(cube, c))
        .collect()
}

/// An enabled recipe with exactly these inputs (in any order), if any: the
/// cube would use that one instead. Returns its description.
pub fn same_recipe(cube: &TxtDoc, inputs: &[String]) -> Option<String> {
    let key = |mut v: Vec<String>| {
        v.sort();
        v.join("|")
    };
    let want = key(inputs.iter().map(|s| normalize_input(s)).collect());
    let cols = input_cols(cube);
    for r in 0..cube.row_count() {
        if cell(cube, r, "enabled").trim() != "1" {
            continue;
        }
        let have = key(cols
            .iter()
            .map(|c| cell(cube, r, c))
            .filter(|v| !v.trim().is_empty())
            .map(|v| normalize_input(&v))
            .collect());
        if !have.is_empty() && have == want {
            let d = cell(cube, r, "description");
            return Some(if d.is_empty() {
                format!("row {}", r + 1)
            } else {
                d
            });
        }
    }
    None
}

/// The level a map item opens, when the template is one: a mod item type with a
/// use action (`pSpell`) whose `len` is a level number (PD2's maps).
pub fn map_item_level(misc: &TxtDoc, row: usize, levels: Option<&TxtDoc>) -> Option<u32> {
    let ty = cell(misc, row, "type").trim().to_lowercase();
    if ty.is_empty() || is_vanilla_type(&ty) || cell(misc, row, "pSpell").trim().is_empty() {
        return None;
    }
    let len = cell(misc, row, "len").trim().to_owned();
    if len.is_empty() || !len.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let levels = levels?;
    let exists = (0..levels.row_count()).any(|r| {
        cell(levels, r, "Id").trim() == len && {
            let d = cell(levels, r, "DrlgType");
            !d.trim().is_empty() && d.trim() != "0"
        }
    });
    exists.then(|| len.parse().unwrap_or(0))
}

/// The level a new map item opens.
#[derive(Clone, Debug)]
pub struct MapTarget {
    /// Levels.txt Id of the map's level.
    pub level_id: u32,
    /// Name shown in game for the level; empty = leave the level's names.
    pub level_title: Option<String>,
}

/// What to make.
#[derive(Clone, Debug)]
pub struct CubeItemInput {
    /// Row of the template item in Misc.txt.
    pub template_row: usize,
    pub name: String,
    pub code: String,
    /// Cube inputs as CubeMain writes them (e.g. `tbk`, `gem4,qty=3`).
    pub inputs: Vec<String>,
    /// Total number of items in the cube (quantities included).
    pub numinputs: u32,
    /// Text that ties the recipe to the map, for the description.
    pub map_name: String,
    /// The user accepted a template of a mod-specific type.
    pub accept_mod_type: bool,
    /// For a map-item template: the level the new item opens.
    pub map: Option<MapTarget>,
}

/// The tables a cube item touches.
#[derive(Clone, Debug)]
pub struct CubeTables {
    pub misc: TxtDoc,
    pub cube: TxtDoc,
    pub levels: Option<TxtDoc>,
    /// The mod's patchstring.tbl.
    pub strings: Option<TblDoc>,
}

/// The edited tables with a change list.
#[derive(Clone, Debug)]
pub struct CubeOutcome {
    pub tables: CubeTables,
    pub changes: Vec<String>,
}

/// Adds the item and its recipe. Refuses a blocked or unconfirmed template, a
/// recipe whose inputs another enabled recipe already uses, an empty or
/// over-full ingredient list, and an item code the table already has.
pub fn plan_cube_item(tables: &CubeTables, input: &CubeItemInput) -> Result<CubeOutcome> {
    let CubeTables { misc, cube, .. } = tables;
    let t = input.template_row;
    if t >= misc.row_count() {
        return refuse("The template item is not in Misc.txt.");
    }
    let risk = template_risk(&cell(misc, t, "type"), &cell(misc, t, "type2"));
    if let Some(TemplateRisk::Blocked(text)) = &risk {
        return refuse(text.clone());
    }
    let opens = map_item_level(misc, t, tables.levels.as_ref());
    if opens.is_some() {
        if input.map.is_none() {
            return refuse(
                "This map isn't a level in the game yet, so a map item can't open it. Add it to the game first.",
            );
        }
        if tables.strings.is_none() {
            return refuse("patchstring.tbl (the mod's strings) could not be read.");
        }
    } else if matches!(risk, Some(TemplateRisk::ModType(_))) && !input.accept_mod_type {
        return refuse("Confirm your mod supports new items of the template's type.");
    }
    if input.code.trim().is_empty() || input.name.trim().is_empty() {
        return refuse("Give the item a name and a code.");
    }
    if (0..misc.row_count()).any(|r| {
        cell(misc, r, "code")
            .trim()
            .eq_ignore_ascii_case(input.code.trim())
    }) {
        return refuse(format!(
            "Misc.txt already has an item with code \"{}\".",
            input.code
        ));
    }
    if input.inputs.is_empty() {
        return refuse("Add at least one ingredient.");
    }
    if input.inputs.len() > 7 {
        return refuse("The cube holds at most 7 kinds of ingredient.");
    }
    if let Some(clash) = same_recipe(cube, &input.inputs) {
        return refuse(format!(
            "Another recipe already uses exactly these ingredients (\"{clash}\"). The cube only runs the first one, so this one would never work. Change the ingredients."
        ));
    }

    let mut out = tables.clone();
    let mut changes = Vec::new();
    let t_code = cell(misc, t, "code");
    let mut values = row_values(misc, t);
    let put = |values: &mut Vec<(String, String)>, k: &str, v: String| {
        if let Some(e) = values.iter_mut().find(|(c, _)| c.eq_ignore_ascii_case(k)) {
            e.1 = v;
        }
    };
    put(&mut values, "name", input.name.clone());
    if has(misc, "*name") {
        put(&mut values, "*name", format!("{} {CAPY_TAG}", input.name));
    }
    put(&mut values, "code", input.code.clone());
    for c in ["normcode", "ubercode", "ultracode"] {
        if cell(misc, t, c).trim() == t_code {
            put(&mut values, c, input.code.clone());
        }
    }
    put(&mut values, "spawnable", "0".into()); // never a random drop
    let mut item_notes = Vec::new();
    let mut strings: Vec<(String, String)> = Vec::new();
    if let (Some(level), Some(map)) = (opens, &input.map) {
        put(&mut values, "len", map.level_id.to_string());
        let key = format!("capy_{}", input.code);
        put(&mut values, "namestr", key.clone());
        strings.push((key.clone(), input.name.clone()));
        item_notes.push(format!(
            "opens level {} (len {level} -> {})",
            map.level_id, map.level_id
        ));
        item_notes.push(format!(
            "its name \"{}\" in patchstring.tbl ({key})",
            input.name
        ));
        if let (Some(title), Some(levels)) = (map.level_title.as_deref(), out.levels.as_mut()) {
            let title = title.trim();
            let row = (0..levels.row_count())
                .find(|&r| cell(levels, r, "Id").trim() == map.level_id.to_string());
            if let (false, Some(lr)) = (title.is_empty(), row) {
                let lk = format!("capy_lvl_{}", map.level_id);
                strings.push((lk.clone(), title.to_owned()));
                for col in ["LevelName", "LevelWarp"] {
                    let from = cell(levels, lr, col);
                    if set(levels, lr, col, &lk)? {
                        changes.push(format!(
                            "Level {}: {col} {} -> {lk}",
                            map.level_id,
                            if from.is_empty() { "(empty)" } else { &from }
                        ));
                    }
                }
                changes.push(format!(
                    "Level {}: shown in game as \"{title}\"",
                    map.level_id
                ));
            }
        }
    }
    append_values(&mut out.misc, &values)?;
    changes.insert(
        0,
        format!(
            "New item \"{}\" (code {}), a copy of \"{}\", added at the end (never drops at random)",
            input.name,
            input.code,
            cell(misc, t, "name")
        ),
    );
    changes.splice(1..1, item_notes);

    let mut recipe: Vec<(String, String)> = vec![
        (
            "description".into(),
            format!(
                "{} [map:{}] {CAPY_TAG}",
                input.name,
                input.map_name.to_lowercase()
            ),
        ),
        ("enabled".into(), "1".into()),
        ("version".into(), "100".into()),
        ("numinputs".into(), input.numinputs.to_string()),
        ("output".into(), input.code.clone()),
    ];
    if opens.is_some() {
        // A map item comes out the way the mod's own map recipes make theirs.
        let maps: Vec<usize> = (0..misc.row_count())
            .filter(|&r| map_item_level(misc, r, tables.levels.as_ref()).is_some())
            .collect();
        let codes = |rows: Vec<usize>| -> Vec<String> {
            rows.iter().map(|&r| cell(misc, r, "code")).collect()
        };
        let same: Vec<String> = codes(
            maps.iter()
                .copied()
                .filter(|&r| cell(misc, r, "type") == cell(misc, t, "type"))
                .collect(),
        );
        let any: Vec<String> = codes(maps.clone());
        let makes = |cs: &[String]| {
            (0..cube.row_count()).find(|&r| {
                cell(cube, r, "enabled") == "1"
                    && cs.contains(
                        &cell(cube, r, "output")
                            .split(',')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_owned(),
                    )
            })
        };
        if let Some(model) = makes(&same).or_else(|| makes(&any)) {
            let output = cell(cube, model, "output");
            let suffix = output.split(',').skip(1).collect::<Vec<_>>().join(",");
            if !suffix.is_empty() {
                recipe[4].1 = format!("{},{suffix}", input.code);
            }
            for col in ["lvl", "plvl", "ilvl"] {
                let v = cell(cube, model, col);
                if !v.is_empty() {
                    recipe.push((col.into(), v));
                }
            }
        }
    }
    for (i, p) in input.inputs.iter().enumerate() {
        recipe.push((format!("input {}", i + 1), p.trim_matches('"').to_owned()));
    }
    let shown = recipe
        .iter()
        .find(|(k, _)| k == "output")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    append_values(&mut out.cube, &recipe)?;
    changes.push(format!("New recipe -> {} (output {shown})", input.name));
    if let Some(tbl) = out.strings.as_mut() {
        for (k, v) in &strings {
            tbl.set(k, v)?;
            changes.push(format!("String {k} = \"{v}\""));
        }
    }
    Ok(CubeOutcome {
        tables: out,
        changes,
    })
}

/// A recipe this tool (or the reference editor) added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaggedRecipe {
    pub row: usize,
    pub description: String,
    pub output: String,
}

fn tagged(s: &str) -> bool {
    s.contains(CAPY_TAG) || s.contains(DS1_STUDIO_TAG)
}

/// Recipes added by these tools (tagged in their description).
pub fn tagged_recipes(cube: &TxtDoc) -> Vec<TaggedRecipe> {
    (0..cube.row_count())
        .filter(|&r| tagged(&cell(cube, r, "description")))
        .map(|r| TaggedRecipe {
            row: r,
            description: cell(cube, r, "description"),
            output: cell(cube, r, "output"),
        })
        .collect()
}

/// Items added by these tools (tagged in `*name`): (row, name, code).
pub fn tagged_items(misc: &TxtDoc) -> Vec<(usize, String, String)> {
    (0..misc.row_count())
        .filter(|&r| tagged(&cell(misc, r, "*name")))
        .map(|r| (r, cell(misc, r, "name"), cell(misc, r, "code")))
        .collect()
}

/// Removes rows by index. Removing items is only safe when no character holds them.
pub fn remove_rows(doc: &TxtDoc, rows: &[usize]) -> Result<TxtDoc> {
    let mut out = doc.clone();
    let mut sorted = rows.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for r in sorted.into_iter().rev() {
        out.delete_row(r)?;
    }
    Ok(out)
}

// ----------------------------------------------------------------- map recipes

/// One ingredient or result of a recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeSlot {
    pub name: String,
    pub code: String,
    pub quantity: u32,
    pub modifiers: Vec<String>,
    pub invfile: Option<String>,
}

/// An enabled recipe that makes a map's item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapRecipe {
    pub row: usize,
    pub description: String,
    pub inputs: Vec<RecipeSlot>,
    pub outputs: Vec<RecipeSlot>,
    pub conditions: Vec<String>,
}

/// Tables for [`find_map_recipes`].
pub struct RecipeTables<'a> {
    pub prest: &'a TxtDoc,
    pub levels: &'a TxtDoc,
    pub misc: &'a TxtDoc,
    pub cube: &'a TxtDoc,
    pub weapons: Option<&'a TxtDoc>,
    pub armor: Option<&'a TxtDoc>,
    pub types: Option<&'a TxtDoc>,
}

fn clean(s: &str) -> String {
    s.replace('"', "").trim().to_owned()
}

/// Follows the map's preset path to its level, then its map items and the
/// enabled recipes that make them. Descriptions are not links.
pub fn find_map_recipes(t: &RecipeTables<'_>, map: &str) -> Vec<MapRecipe> {
    let want = crate::txt_util::rel_key(&clean(map));
    let mut ids = std::collections::BTreeSet::new();
    let mut claimed = std::collections::BTreeSet::new();
    for r in 0..t.prest.row_count() {
        let id = num(&cell(t.prest, r, "LevelId"));
        if id == 0 || !claimed.insert(id) {
            continue;
        }
        if (1..=6).any(|i| {
            crate::txt_util::rel_key(&clean(&cell(t.prest, r, &format!("File{i}")))) == want
        }) {
            ids.insert(id as u32);
        }
    }
    let codes: std::collections::BTreeSet<String> = (0..t.misc.row_count())
        .filter(|&r| map_item_level(t.misc, r, Some(t.levels)).is_some_and(|id| ids.contains(&id)))
        .map(|r| clean(&cell(t.misc, r, "code")).to_lowercase())
        .collect();
    let mut names = std::collections::HashMap::new();
    let mut icons = std::collections::HashMap::new();
    for doc in [t.types, t.weapons, t.armor, Some(t.misc)]
        .into_iter()
        .flatten()
    {
        for r in 0..doc.row_count() {
            let code = clean(&cell(doc, r, "code")).to_lowercase();
            let mut name = clean(&cell(doc, r, "name"));
            if name.is_empty() {
                name = clean(&cell(doc, r, "ItemType"));
            }
            if !code.is_empty() && !name.is_empty() {
                names.insert(code.clone(), name);
            }
            let inv = clean(&cell(doc, r, "invfile"));
            if !code.is_empty() && !inv.is_empty() {
                icons.insert(code, inv);
            }
        }
    }
    let slot = |raw: &str| -> RecipeSlot {
        let cleaned = clean(raw);
        let mut parts = cleaned.split(',').map(|s| s.trim().to_owned());
        let code = parts.next().unwrap_or_default();
        let params: Vec<String> = parts.collect();
        let qty = params.iter().find(|p| {
            p.to_lowercase()
                .strip_prefix("qty=")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        });
        let lower = code.to_lowercase();
        RecipeSlot {
            name: names.get(&lower).cloned().unwrap_or_else(|| code.clone()),
            quantity: qty.and_then(|q| q[4..].parse().ok()).unwrap_or(1),
            modifiers: params
                .iter()
                .filter(|p| !p.is_empty() && Some(*p) != qty)
                .cloned()
                .collect(),
            invfile: icons.get(&lower).cloned(),
            code,
        }
    };
    let mut out = Vec::new();
    for r in data_rows(t.cube) {
        if cell(t.cube, r, "enabled").trim() != "1" {
            continue;
        }
        let outputs: Vec<RecipeSlot> = ["output", "output b", "output c"]
            .iter()
            .map(|c| cell(t.cube, r, c))
            .filter(|s| !clean(s).is_empty())
            .map(|s| slot(&s))
            .collect();
        if !outputs
            .iter()
            .any(|s| codes.contains(&s.code.to_lowercase()))
        {
            continue;
        }
        let mut conditions = Vec::new();
        let diff = cell(t.cube, r, "diff");
        if !diff.trim().is_empty() {
            let name = ["Normal", "Nightmare", "Hell"]
                .get(num(&diff) as usize)
                .map(|s| (*s).to_owned());
            conditions.push(format!(
                "Difficulty: {}",
                name.unwrap_or_else(|| diff.trim().to_owned())
            ));
        }
        if cell(t.cube, r, "ladder").trim() == "1" {
            conditions.push("Ladder only".into());
        }
        let class = cell(t.cube, r, "class");
        if !class.trim().is_empty() {
            conditions.push(format!("Class: {class}"));
        }
        if num(&cell(t.cube, r, "op")) != 0 {
            conditions.push(format!(
                "Additional game condition: {} / {} / {}",
                cell(t.cube, r, "op"),
                cell(t.cube, r, "param"),
                cell(t.cube, r, "value")
            ));
        }
        let description = cell(t.cube, r, "description");
        out.push(MapRecipe {
            row: r,
            description: if description.is_empty() {
                format!("Recipe {}", r + 1)
            } else {
                description
            },
            inputs: (1..=7)
                .map(|i| cell(t.cube, r, &format!("input {i}")))
                .filter(|s| !clean(s).is_empty())
                .map(|s| slot(&s))
                .collect(),
            outputs,
            conditions,
        });
    }
    out
}
