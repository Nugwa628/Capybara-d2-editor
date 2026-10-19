# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added
- **P0 complete.** Acceptance (vanilla 1.14d + Median XL): every DS1 round-trips
  byte-identically (2,276 + 2,424), every DT1 parses (six unreferenced 4.1 files
  excepted), every known name resolves, `.bin` equals `.txt`, every Median XL
  LvlTypes/LvlPrest reference resolves.
- `d2-formats`: palettes (`pal.dat`, PL2), DT1 tiles, DC6, DCC, COF, AnimData.
- `d2-assets`: known-names harvest, `load_excel` (`.bin`/`.txt` by game version).
- `capy-history`: atomic saves with `CHANGED.txt` and mark-as-packed, deduplicated
  snapshot store with retention, gix milestones; crash-safe (kill test).
- `scripts/oracle_renders.py`: reference renders for the P1 oracle test.
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
