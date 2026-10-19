# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added
- `d2-assets`: read-only MPQ reader (v1-v4 headers, encryption and FIX_KEY,
  implode, zlib, bzip2), loose-folder and stub CASC sources, an N-layer
  overlay, TOML game profiles (Vanilla, Median XL), a known-names container
  with an embedded seed list, and an `extract` example.
- `d2-formats`: DS1 reader/writer for every version (byte-identical round
  trip on all vanilla and Median XL maps), `.txt` tables, compiled excel
  `.bin` decoder (LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset, Levels),
  typed LvlTypes/LvlPrest/Levels views, `.tbl` string tables.
- `d2-assets` wave 2: `KnownNames::harvest` (listfiles, seed, LvlTypes/LvlPrest
  file columns and every DS1's tileset references, to a fixed point, keeping only
  names that resolve), `tables::load_excel` (`.bin`/`.txt` per game version),
  `GameVersion::excel`, and the P0 acceptance tests (DS1 round trip, names resolve,
  `.bin` vs `.txt`, Median XL references).
- `scripts/extract_fixtures.py`: independent Python MPQ extractor used to
  cross-check the Rust reader.
- `docs/specs/`: reference guides cross-checked against the reference source.
- Cargo workspace skeleton (`d2-formats`, `d2-assets`, `capy-index`, `capy-edit`,
  `capy-history`, `capy-app`), LICENSE (MIT) and NOTICE.
- `d2-formats` wave 1: `Grid`, `IndexedImage`, DS1 reader/writer (every version,
  byte-identical round trip on the full vanilla and Median XL corpora), `.txt`
  tables, excel `.bin` decoders (LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset,
  Levels), typed level tables and `.tbl` string tables.
