//! Plain-language explanations of D2 `.txt` columns, shown behind "?" in
//! the table editor. The wording follows ds1-studio's `src/data/columnHelp.ts`
//! (MIT; credited in NOTICE). A pattern matches a column name ignoring case;
//! `#` stands for one or more digits and `[abc]` / `[1-6]` for one character
//! of a set.

type Help = &'static [(&'static str, &'static str)];

const LVLPREST: Help = &[
    (
        "Name",
        "A label for this preset map. Only used by people reading the table; the game ignores it.",
    ),
    (
        "Def",
        "The preset's ID number. Other tables and the game's code refer to the preset by this number, so every row needs a unique one.",
    ),
    (
        "LevelId",
        "Which level (Id in Levels.txt) this preset IS. 0 means the preset is a building block that a level generator places inside other levels (rooms of a dungeon, pieces of wilderness).",
    ),
    (
        "Populate",
        "1 = the game may spawn random monsters in this map.",
    ),
    (
        "Logicals",
        "1 = walls and floors block walking and line of sight as usual. 0 = everything is walkable (rarely wanted).",
    ),
    (
        "Outdoors",
        "1 = outdoor area: day/night lighting and weather apply.",
    ),
    (
        "Animate",
        "1 = animated tiles (water, lava) in this map animate.",
    ),
    (
        "KillEdge",
        "1 = the tiles along the map border are removed, so the map blends into what surrounds it.",
    ),
    (
        "FillBlanks",
        "1 = empty cells are filled with the level's default ground tile instead of being left black.",
    ),
    (
        "SizeX",
        "Width of the preset in tiles. 0 = take it from the DS1 file.",
    ),
    (
        "SizeY",
        "Height of the preset in tiles. 0 = take it from the DS1 file.",
    ),
    ("AutoMap", "1 = the map shows on the automap once explored."),
    (
        "Scan",
        "1 = this map can be \"scanned\" for warps/exits, needed for maps with entrances to other levels.",
    ),
    (
        "Pops",
        "How many \"pop pads\" (areas that reveal hidden parts, like building roofs disappearing when you walk in) the map has.",
    ),
    ("PopPad", "Size of the pop pad areas."),
    (
        "Files",
        "How many DS1 variations (File1..File6) this preset has. The game picks one of them at random each game.",
    ),
    (
        "File[1-6]",
        "A DS1 map file for this preset, relative to data\\global\\tiles (e.g. Act1\\Town\\TownN1.ds1). If several are filled in, the game picks one at random.",
    ),
    (
        "Dt1Mask",
        "Which tile libraries of the level type to load, as a bit mask: bit 0 = \"File 1\" in LvlTypes.txt, bit 1 = \"File 2\", and so on. A tile whose DT1 is not in the mask is invisible in game.",
    ),
    ("Beta", "Unused leftover from development."),
    (
        "Expansion",
        "1 = Lord of Destruction row. Only matters for the separator lines the game uses between classic and expansion rows.",
    ),
];

const LVLTYPES: Help = &[
    (
        "Name",
        "A label for this level type (tile set). Only for people reading the table.",
    ),
    (
        "Id",
        "The level type's ID. Levels.txt LevelType points here.",
    ),
    (
        "File #",
        "One tile library (DT1 file) this level type can use, relative to data\\global\\tiles. A preset picks which of these to load with its Dt1Mask (bit N-1 = File N). 0 = empty slot.",
    ),
    ("Beta", "Unused leftover from development."),
    (
        "Act",
        "Which act this tile set belongs to (1-5). Used for loading; the level's own Act decides the palette.",
    ),
    ("Expansion", "1 = Lord of Destruction row."),
];

const LEVELS: Help = &[
    (
        "Name",
        "Internal name of the level (used by other tables and for debugging).",
    ),
    (
        "Id",
        "The level's ID number. LvlPrest LevelId, Vis0-7 and many game systems refer to this number.",
    ),
    (
        "Pal",
        "Which act palette (0-4 = Act 1-5) the level's graphics use.",
    ),
    (
        "Act",
        "Which act the level is in (0-4 = Act 1-5): decides the palette, the town players return to and act-wide effects.",
    ),
    (
        "QuestFlag",
        "A quest that must be done before players can enter (classic).",
    ),
    (
        "QuestFlagEx",
        "A quest that must be done before players can enter (expansion).",
    ),
    (
        "Layer",
        "Automap layer: levels that share a layer share one automap.",
    ),
    (
        "SizeX",
        "Width of the level in tiles (Normal). For preset levels it should match the DS1.",
    ),
    ("SizeX(N)", "Width of the level in tiles (Nightmare)."),
    ("SizeX(H)", "Width of the level in tiles (Hell)."),
    ("SizeY", "Height of the level in tiles (Normal)."),
    ("SizeY(N)", "Height of the level in tiles (Nightmare)."),
    ("SizeY(H)", "Height of the level in tiles (Hell)."),
    (
        "OffsetX",
        "Where this level sits in the act's world, horizontally (tiles). Levels that connect directly must line up.",
    ),
    (
        "OffsetY",
        "Where this level sits in the act's world, vertically (tiles).",
    ),
    ("Depend", "Another level this one is placed relative to."),
    (
        "Teleport",
        "Whether teleport works here: 0 = yes, 1 = no, 2 = only in line of sight.",
    ),
    ("Rain", "1 = rain/snow falls here (outdoor levels)."),
    ("Mud", "1 = footsteps leave mud splashes."),
    ("NoPer", "1 = no perspective mode."),
    (
        "LOSDraw",
        "1 = walls block the view (line of sight) when drawing.",
    ),
    (
        "FloorFilter",
        "1 = floors are drawn with a lighting filter.",
    ),
    ("BlankScreen", "1 = unexplored parts are black."),
    ("DrawEdges", "1 = the edges of the map are drawn."),
    (
        "IsInside",
        "1 = indoor level (no weather, different lighting).",
    ),
    (
        "DrlgType",
        "How the level is built: 1 = maze of rooms (LvlMaze.txt), 2 = one preset map (LvlPrest.txt), 3 = random outdoor area (LvlSub.txt substitutions).",
    ),
    (
        "LevelType",
        "Which tile set (Id in LvlTypes.txt) the level uses.",
    ),
    (
        "SubType",
        "Which LvlSub.txt substitution type outdoor areas use.",
    ),
    ("SubTheme", "Which LvlSub theme (variation) is used."),
    (
        "SubWaypoint",
        "LvlSub type used to place the waypoint in random outdoor levels.",
    ),
    (
        "SubShrine",
        "LvlSub type used to place shrines in random outdoor levels.",
    ),
    (
        "Vis#",
        "A level this one connects to (Id in Levels.txt), e.g. the next dungeon floor. Up to 8 connections.",
    ),
    (
        "Warp#",
        "Which LvlWarp.txt entry (the doorway/stairs graphic) the matching Vis connection uses. -1 = none.",
    ),
    ("Intensity", "Ambient light strength."),
    ("Red", "Ambient light colour (red part)."),
    ("Green", "Ambient light colour (green part)."),
    ("Blue", "Ambient light colour (blue part)."),
    ("Portal", "1 = town portals can be cast here."),
    (
        "Position",
        "1 = players entering are placed at a special tile position.",
    ),
    (
        "SaveMonsters",
        "1 = killed monsters stay dead when you come back in the same game.",
    ),
    ("Quest", "Quest attached to this level."),
    ("WarpDist", "How close you must be to a warp to use it."),
    (
        "MonLvl[123]",
        "Monster level in Normal / Nightmare / Hell (classic).",
    ),
    (
        "MonLvl[123]Ex",
        "Monster level in Normal / Nightmare / Hell (expansion).",
    ),
    ("MonDen", "Monster density: how many monster groups spawn."),
    ("MonUMin", "Minimum number of unique/champion packs."),
    ("MonUMax", "Maximum number of unique/champion packs."),
    ("MonWndr", "1 = monsters wander between rooms."),
    (
        "MonSpcWalk",
        "Distance for special monster walking behaviour.",
    ),
    (
        "NumMon",
        "How many different monster types (mon1..mon25) spawn in one game.",
    ),
    (
        "mon#",
        "A monster type (Id in MonStats.txt) that can spawn here in Normal (classic).",
    ),
    (
        "nmon#",
        "A monster type that can spawn here in Nightmare/Hell.",
    ),
    (
        "umon#",
        "A monster type that can spawn as a unique/champion leader.",
    ),
    ("rangedspawn", "1 = ranged monsters spawn at a distance."),
    (
        "cmon[0-9]",
        "A critter (harmless ambient creature) that spawns here.",
    ),
    ("cpct[0-9]", "Chance for the matching critter to spawn."),
    ("camt[0-9]", "How many of the matching critter spawn."),
    (
        "Themes",
        "Which room themes (special rooms like shrines or treasure) can appear.",
    ),
    (
        "SoundEnv",
        "Which sound environment (echo, reverb) the level uses.",
    ),
    (
        "Waypoint",
        "Waypoint slot for this level. 255 = no waypoint.",
    ),
    ("LevelName", "The name shown in game (string table key)."),
    (
        "LevelWarp",
        "The text shown when hovering the entrance to this level.",
    ),
    ("EntryFile", "The loading-screen image used when entering."),
    (
        "ObjGrp#",
        "An object group (ObjGroup.txt) that spawns here: barrels, shrines, chests...",
    ),
    (
        "ObjPrb#",
        "Chance (%) that the matching object group spawns.",
    ),
    ("Beta", "Unused leftover from development."),
];

const LVLWARP: Help = &[
    ("Name", "Label for this warp (doorway/stairs) type."),
    ("Id", "Warp ID; Levels.txt Warp0-7 point here."),
    (
        "SelectX",
        "Horizontal offset of the clickable area of the warp.",
    ),
    ("SelectY", "Vertical offset of the clickable area."),
    ("SelectDX", "Width of the clickable area."),
    ("SelectDY", "Height of the clickable area."),
    (
        "ExitWalkX",
        "Where the player walks to when leaving through the warp (x).",
    ),
    (
        "ExitWalkY",
        "Where the player walks to when leaving through the warp (y).",
    ),
    (
        "OffsetX",
        "Where the player appears after coming through (x).",
    ),
    (
        "OffsetY",
        "Where the player appears after coming through (y).",
    ),
    (
        "LitVersion",
        "1 = the warp has a highlighted version when hovered.",
    ),
    (
        "Tiles",
        "Tile sub-index of the warp's special tiles in the DS1.",
    ),
    ("Direction", "Which way the warp faces."),
];

const LVLMAZE: Help = &[
    ("Name", "Label for this maze level."),
    ("Level", "The level (Id in Levels.txt) built as a maze."),
    ("Rooms", "Number of rooms (Normal)."),
    ("Rooms(N)", "Number of rooms (Nightmare)."),
    ("Rooms(H)", "Number of rooms (Hell)."),
    ("SizeX", "Width of each room in tiles."),
    ("SizeY", "Height of each room in tiles."),
    (
        "Merge",
        "Chance that neighbouring rooms merge into bigger ones.",
    ),
];

const LVLSUB: Help = &[
    ("Name", "Label for this substitution."),
    (
        "Type",
        "Substitution type: Levels.txt SubType picks which rows apply.",
    ),
    (
        "File",
        "The DS1 whose substitution groups provide the replacement pieces.",
    ),
    ("CheckAll", "1 = all cells must match before substituting."),
    ("BordType", "Border type the substitution applies to."),
    ("GridSize", "Size of the grid pieces are placed on."),
    ("Prob#", "Chance for the matching substitution group."),
    ("Trials#", "How many times the game tries to place it."),
    ("Max#", "Maximum number placed."),
];

const MONPRESET: Help = &[
    (
        "Act",
        "Act (1-5) this list belongs to. DS1 NPC objects (type 1) are numbered within their act, in the order of these rows.",
    ),
    (
        "Place",
        "What a type-1 object with this number spawns: a monster (MonStats Id), a super unique, or a special spawn point.",
    ),
];

const OBJGROUP: Help = &[
    ("GroupName", "Label for this object group."),
    ("ID#", "An object (Objects.txt Id) in the group."),
    ("DENSITY#", "How densely the matching object spawns."),
    ("PROB#", "Chance for the matching object."),
    ("SHRINES", "1 = the group spawns shrines."),
    ("WELLS", "1 = the group spawns wells."),
];

const CUBEMAIN: Help = &[
    (
        "description",
        "A label for the recipe (for people reading the table).",
    ),
    ("enabled", "1 = the recipe works."),
    ("ladder", "1 = only on ladder realms."),
    (
        "min diff",
        "Lowest difficulty the recipe works in (0 Normal, 1 Nightmare, 2 Hell).",
    ),
    ("version", "0 = classic, 100 = Lord of Destruction."),
    (
        "op",
        "An optional condition checked on the player/item (with param/value).",
    ),
    ("param", "Parameter for op."),
    ("value", "Value for op."),
    ("class", "Only this character class can use the recipe."),
    (
        "numinputs",
        "Total number of items that must be in the cube.",
    ),
    (
        "input #",
        "An ingredient: an item code or type, optionally with qualifiers like \"qty=2\", \"nor\", \"mag\", \"rar\", \"uni\".",
    ),
    (
        "output",
        "What the recipe creates: an item code, \"usetype\", \"useitem\", or a special output like \"Cow Portal\".",
    ),
    ("output [bc]", "An additional output."),
    ("lvl", "Item level of the output (0 = default)."),
    (
        "plvl",
        "Output item level from the player's level (percent).",
    ),
    (
        "ilvl",
        "Output item level from the input item's level (percent).",
    ),
    ("mod #", "A property added to the output item."),
    ("mod # chance", "Chance for the property."),
    ("mod # param", "Parameter for the property."),
    ("mod # min", "Minimum value of the property."),
    ("mod # max", "Maximum value of the property."),
];

const MISC: Help = &[
    ("name", "Name for people reading the table."),
    (
        "code",
        "The item's unique 3-4 character code. Cube recipes and other tables refer to items by this code.",
    ),
    ("namestr", "String-table key of the name shown in game."),
    ("level", "Item level (affects what can drop / spawn on it)."),
    ("levelreq", "Character level needed to use it."),
    (
        "spawnable",
        "1 = can be created by the game (drops, vendors).",
    ),
    ("cost", "Base price."),
    ("invfile", "Inventory graphic (DC6 file name)."),
    ("flippyfile", "Graphic shown when the item drops."),
    (
        "type",
        "Item type (ItemTypes.txt code), which recipes and scripts use.",
    ),
    ("stackable", "1 = stacks in the inventory."),
];

const OBJECTS: Help = &[
    ("Name", "Name of the object (for people reading the table)."),
    (
        "Id",
        "Object ID. DS1 object entries (type 2) are converted to these through per-act tables.",
    ),
    ("Token", "Two-letter folder code of the object's graphics."),
    ("SizeX", "Footprint width in sub-tiles."),
    ("SizeY", "Footprint height in sub-tiles."),
    (
        "OperateFn",
        "What happens when a player clicks it (chest, shrine, door, waypoint...).",
    ),
    ("Selectable0", "1 = can be clicked in its first mode."),
];

fn table(name: &str) -> Option<Help> {
    let n = name.to_ascii_lowercase();
    let n = n.strip_suffix(".txt").unwrap_or(&n);
    Some(match n {
        "lvlprest" => LVLPREST,
        "lvltypes" => LVLTYPES,
        "levels" => LEVELS,
        "lvlwarp" => LVLWARP,
        "lvlmaze" => LVLMAZE,
        "lvlsub" => LVLSUB,
        "monpreset" => MONPRESET,
        "objgroup" => OBJGROUP,
        "cubemain" => CUBEMAIN,
        "misc" => MISC,
        "objects" => OBJECTS,
        _ => return None,
    })
}

/// Does `text` match `pat` entirely (ignoring case)?
fn matches(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'#') => {
            let digits = text.iter().take_while(|c| c.is_ascii_digit()).count();
            (1..=digits).any(|n| matches(&pat[1..], &text[n..]))
        }
        Some(b'[') => {
            let Some(end) = pat.iter().position(|&c| c == b']') else {
                return false;
            };
            let set = &pat[1..end];
            let Some(&c) = text.first() else { return false };
            let c = c.to_ascii_lowercase();
            let mut i = 0;
            let mut hit = false;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == b'-' {
                    hit |= (set[i].to_ascii_lowercase()..=set[i + 2].to_ascii_lowercase())
                        .contains(&c);
                    i += 3;
                } else {
                    hit |= set[i].to_ascii_lowercase() == c;
                    i += 1;
                }
            }
            hit && matches(&pat[end + 1..], &text[1..])
        }
        Some(&p) => {
            text.first().is_some_and(|&c| c.eq_ignore_ascii_case(&p))
                && matches(&pat[1..], &text[1..])
        }
    }
}

/// A plain-language explanation of a column, or `None` when there is none.
pub fn column_help(table_name: &str, column: &str) -> Option<&'static str> {
    table(table_name)?
        .iter()
        .find(|(pat, _)| matches(pat.as_bytes(), column.trim().as_bytes()))
        .map(|(_, text)| *text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_like_the_original_regexes() {
        assert!(column_help("LvlTypes.txt", "File 12").is_some());
        assert!(column_help("lvltypes", "File").is_none());
        assert!(column_help("LvlPrest", "File3").is_some());
        assert!(column_help("LvlPrest", "File7").is_none());
        assert!(column_help("Levels", "SizeX(N)").is_some());
        assert!(column_help("Levels", "Vis7").is_some());
        assert!(column_help("Levels", "MonLvl2Ex").is_some());
        assert!(column_help("CubeMain", "mod 3 chance").is_some());
        assert!(column_help("CubeMain", "output b").is_some());
        assert!(column_help("Unknown", "Name").is_none());
    }
}
