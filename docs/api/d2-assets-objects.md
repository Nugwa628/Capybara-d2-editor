# `d2_assets::objects` public API (owner: lead)

Resolves DS1 objects (type 1 = monsters/NPCs via MonPreset, type 2 = objects
via the act×150 remap) to what is needed to draw and name them. Used by P5
object rendering and the app's object picker.

```rust
// lib.rs: pub mod objects; pub use objects::{ObjectCatalog, ObjectInfo, ObjectClass, SpriteBase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectClass { Npc /* DS1 type 1 */, Object /* type 2 */, Other(i32) }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpriteBase { Objects /* data\global\objects */, Monsters /* data\global\monsters */ }

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
    pub size: (i32, i32),             // SizeX/SizeY
    pub offset: (i32, i32),           // Xoffset/Yoffset
    pub draw_effect: Option<u8>,      // object-level transparency/blend if the table has one
    pub frame_count: Option<u32>,     // FrameCnt of the mode
    pub frame_delta: Option<u32>,     // FrameDelta of the mode (animation speed, 256 = 1 frame per tick)
    pub source: &'static str,         // "objpreset" | "obj.txt" | "monpreset" | "unresolved"
}

pub struct ObjectCatalog { /* tables */ }
impl ObjectCatalog {
    /// Loads Objects, ObjPreset (when the profile has it; else the embedded
    /// MIT `obj.txt` from DS1Edit, credited in NOTICE), MonPreset/MonStats/
    /// MonStats2 when available as .txt or decodable .bin, and string tables.
    /// Missing tables become warnings, never errors.
    pub fn build(overlay: &Overlay, version: GameVersion) -> Result<(ObjectCatalog, Vec<String>)>;
    /// `act` is the DS1's effective act (0..=4); `ds1_version` selects the
    /// pre-v5/pre-v6 id rules from docs/specs/ds1.md.
    pub fn resolve(&self, act: u8, ds1_version: i32, object: &d2_formats::ds1::Object) -> ObjectInfo;
    /// Every placeable (kind, id) for an act, for the object picker.
    pub fn placeable(&self, act: u8) -> Vec<ObjectInfo>;
}
```
