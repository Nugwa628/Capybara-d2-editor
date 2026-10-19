# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added
- Cargo workspace skeleton (`d2-formats`, `d2-assets`, `capy-index`, `capy-edit`,
  `capy-history`, `capy-app`), LICENSE (MIT) and NOTICE.
- `d2-formats` wave 1: `Grid`, `IndexedImage`, DS1 reader/writer (every version,
  byte-identical round trip on the full vanilla and Median XL corpora), `.txt`
  tables, excel `.bin` decoders (LvlTypes, LvlPrest, LvlWarp, Objects, ObjPreset,
  Levels), typed level tables and `.tbl` string tables.
