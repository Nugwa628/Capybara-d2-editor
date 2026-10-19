# `d2_assets::presets` public API (owner: lead)

Answers "how does the game load this DS1?": which LvlPrest row(s) name it,
which Level and LvlType they belong to, the act, the palette, and the ordered,
Dt1Mask-filtered DT1 list. Also imports DS1Edit's legacy `assets/*.ini`
(`<LvlTypesId> <LvlPrestDef> <path>` per line) as overrides.

Guessing rule (lead decision, 2026-10-19): wherever a LvlType must be guessed
(LvlPrest rows with LevelId 0 or an empty LvlType, PathGuess, TileCoverage),
candidates rank by tile coverage, then overlap with the DS1's own tileset
list, then directory match, then lowest id. `build` may read DS1 bytes for
these candidates.

```rust
// lib.rs: pub mod presets; pub use presets::{MapCatalog, MapInfo, MapInfoSource, LegacyEntry, parse_legacy_ini};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapInfoSource {
    /// User override (e.g. imported legacy ini, or picked in the UI).
    Override,
    /// A LvlPrest row's File1..6 names this DS1; LvlType via Levels(LevelId).LevelType.
    LvlPrest,
    /// No LvlPrest row; LvlType chosen because its File slots share the DS1's
    /// directory (most shared DT1s wins; ties → lowest id). All File slots used.
    PathGuess,
    /// No LvlPrest row and no directory candidate: the LvlType whose DT1 set
    /// resolves the most of the DS1's cells, accepted at coverage >= 0.90.
    TileCoverage,
    /// Nothing matched: act from the DS1 header, no DT1s.
    Ds1Header,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapInfo {
    pub ds1: GamePath,
    pub source: MapInfoSource,
    pub lvlprest_def: Option<u32>,
    pub lvlprest_slot: Option<u8>,          // which File1..6 (0-based)
    pub level_id: Option<u32>,
    pub level_name: Option<String>,         // Levels LevelName
    pub lvl_type: Option<u32>,
    /// 0-based act: LvlType act, else Level act, else the DS1 header's effective_act.
    pub act: u8,
    /// 0-based act whose `pal.dat` to use: `act`, except LvlType id 35 uses Act 4 (3),
    /// as the reference editor does.
    pub palette_act: u8,
    /// DT1s in load order (LvlType File slot order, masked by Dt1Mask when known).
    pub dt1_files: Vec<GamePath>,
    /// DT1s named above that do not resolve in the overlay.
    pub missing_dt1: Vec<GamePath>,
    /// Fraction of visible non-empty cells whose tile key resolves in
    /// `dt1_files`, when it was computed (guessed LvlTypes).
    pub coverage: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyEntry { pub lvl_type: u32, pub lvlprest_def: u32, pub ds1: GamePath }
/// Parses one legacy ini. `assets/tiles/ACT1/TOWN/townE1.ds1` maps to
/// `data\global\tiles\act1\town\towne1.ds1`. Bad lines are returned as warnings.
pub fn parse_legacy_ini(text: &str) -> (Vec<LegacyEntry>, Vec<String>);

pub struct MapCatalog { /* lvl types, prests, levels, by-path index, overrides */ }
impl MapCatalog {
    /// Loads LvlTypes, LvlPrest and Levels with tables::load_excel. `ds1_names`
    /// is the set of DS1 paths to describe (usually KnownNames filtered to .ds1).
    pub fn build(overlay: &Overlay, version: GameVersion, ds1_names: &[GamePath]) -> Result<(MapCatalog, Vec<String>)>;
    /// Every interpretation of this DS1, best first (Override > LvlPrest > PathGuess > Ds1Header;
    /// among several LvlPrest rows, lowest Def first). Never empty for a known DS1.
    pub fn lookup(&self, ds1: &GamePath) -> Vec<MapInfo>;
    pub fn best(&self, ds1: &GamePath) -> Option<MapInfo>;
    /// The DS1Edit command-line form: an explicit LvlType id and LvlPrest Def.
    pub fn resolve_explicit(&self, ds1: &GamePath, lvl_type: u32, lvlprest_def: Option<u32>) -> Result<MapInfo>;
    pub fn set_override(&mut self, ds1: GamePath, lvl_type: u32, lvlprest_def: Option<u32>);
    pub fn clear_override(&mut self, ds1: &GamePath);
    /// Applies legacy entries as overrides; returns how many matched a known DS1.
    pub fn import_legacy(&mut self, entries: &[LegacyEntry]) -> usize;
    pub fn overrides(&self) -> Vec<(GamePath, u32, Option<u32>)>;
    pub fn lvl_type_ids(&self) -> Vec<u32>;
}
```
