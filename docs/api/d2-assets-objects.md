# `d2_assets::objects` public API (owner: lead)

Resolves DS1 objects (type 1 = monsters/NPCs via MonPreset, type 2 = objects
via the act×150 remap) to what is needed to draw and name them. Used by P5
object rendering and the app's object picker.

```rust
// lib.rs: pub mod objects; pub use objects::{ObjectCatalog, ObjectInfo, ObjectClass, PaletteShift, SpriteBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectClass { Npc /* DS1 type 1 */, Object /* type 2 */, Other(i32) }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteBase { Objects /* data\global\objects */, Monsters /* data\global\monsters */ }

/// A unique/special monster's palette shift: DS1Edit obj.txt `Colormap` + `Index`
/// (cof.c:216-242). The file is a run of 256-byte rows (RandTransforms.dat: 30,
/// a monster's COF\Palshift.dat: 8); the row maps palette index -> index.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaletteShift {
    pub path: String, // as obj.txt names it, e.g. "Data\Global\Monsters\RandTransforms.dat"
    pub row: u32,     // obj.txt Index - 1 (Index 0/empty = no shift, never stored)
}
impl PaletteShift {
    /// The row in the file's bytes; None when the file is too short (the reference then draws unshifted).
    pub fn row_in<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8; 256]>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectInfo {
    pub class: ObjectClass,
    pub ds1_kind: i32,
    pub ds1_id: i32,
    /// Objects.txt row (type 2) or MonStats row (type 1) when known.
    pub row: Option<u32>,
    /// Display name: string-table lookup of the row's name key when possible,
    /// else the table's Name column, else "type N id M".
    pub name: String,
    pub sprite_base: Option<SpriteBase>,
    pub token: Option<String>,        // COF token, e.g. "TO", "AF"
    pub mode: String,                 // mode token used for drawing (objects: per Objects.txt/obj.txt; monsters "NU")
    pub weapon_class: String,         // "HTH" unless the data says otherwise
    pub order_flag: u8,               // Objects.txt OrderFlag of that mode (0/1/2); monsters 0
    pub parts: Vec<String>,           // enabled components (HD TR LG RA LA RH LH SH S1..S8)
    pub skins: Vec<(String, String)>, // per-component skin token from obj.txt, e.g. ("TR","LIT"); missing = "LIT"
    pub direction: Option<u8>,        // default facing from obj.txt `Direction` (renderer default when no override)
    // Type 1 via MonPreset (DS1 v5+): skins, direction and colormap come from the obj.txt row
    // (act, 1, id) when it draws the same token as the MonStats route (as DS1Edit draws it).
    pub size: (i32, i32),             // SizeX/SizeY
    pub offset: (i32, i32),           // Xoffset/Yoffset
    pub draw_effect: Option<u8>,      // object-level transparency/blend if the table has one
    pub frame_count: Option<u32>,     // FrameCnt of the mode
    pub frame_delta: Option<u32>,     // FrameDelta of the mode (animation speed, 256 = 1 frame per tick)
    /// obj.txt palette shift of row (act, type, id), only when that row draws the same token
    /// (type 1 needs DS1 v5+). Renderers map layers with trans_a == 0 through it (cof.c:343-361).
    pub colormap: Option<PaletteShift>,
    pub source: &'static str,         // "objpreset" | "obj.txt" | "monpreset" | "unresolved"
}

pub struct ObjectCatalog { /* tables */ }
impl ObjectCatalog {
    /// Loads Objects, ObjPreset (when the profile has it; else the embedded
    /// MIT `obj.txt` from DS1Edit, credited in NOTICE), MonPreset/MonStats/
    /// MonStats2/SuperUniques through `tables::load_excel` (`.bin` first on
    /// 1.13c), and string tables.
    /// Missing tables become warnings, never errors.
    pub fn build(overlay: &Overlay, version: GameVersion) -> Result<(ObjectCatalog, Vec<String>)>;
    /// `act` is the DS1's effective act (0..=4); `ds1_version` selects the
    /// pre-v5/pre-v6 id rules from docs/specs/ds1.md.
    pub fn resolve(&self, act: u8, ds1_version: i32, object: &d2_formats::ds1::Object) -> ObjectInfo;
    /// Every placeable (kind, id) for an act, for the object picker.
    pub fn placeable(&self, act: u8) -> Vec<ObjectInfo>;
}
// Notes: the game rewrites six act 3/5 type-1 presets into objects
// (297→382, 366→404, 514→461, 537/538/539→476/475/474), so a type-1 DS1
// object can resolve with `class: Object`. Vanilla rows 580-582 referenced by
// the act×150 table do not exist in the 573-row Objects.txt (unresolved, as in
// the game). Type 1 follows MonPreset -> MonStats (or SuperUniques -> its
// MonStats class) -> MonStats2 for both forms; the compiled `.bin` form has no
// Id text, so a monster whose NameStr is blank/"dummy" is named "monster <row>"
// (Median XL: 89.6% of its type 1 objects have a token; the rest are MonPlace
// markers, as in vanilla).
```
