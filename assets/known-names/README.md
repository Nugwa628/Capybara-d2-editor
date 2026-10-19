# Known-names seed list

`seed.txt` lists Diablo II game file paths that the game or the editor loads by hard-coded name, so no data table refers to them. `Patch_D2.mpq` has no listfile, so the editor uses this list to look those files up by name.

Format: one game-relative path per line, lowercase, backslash separators, no leading backslash. `#` starts a comment and blank lines are allowed. Each `# --- section ---` block is sorted and free of duplicates.

Sources: the reference repo's `docs/guides/*.md` (excel inventory in `excel-tables-and-data-loading.md`, palettes in `palettes-and-colour.md`, string tables in `tbl-string-tables.md`) and `src/` string literals (`Data\\Global\\...`, `Data\\Local\\...`). Only concrete file names are included. `%s`/`*` patterns are expanded only where the set is small and known (`act1`..`act5`, `menu0`..`menu4`).

Excel tables are listed as both `.txt` and `.bin`, because the loader can read either (see the excel guide, "The `.txt`/`.bin` fork").

Not from the reference: `automap.*` and the `# --- ... not named in reference (unverified) ---` blocks are common vanilla table names added from D2 convention. Verify them against an archive before relying on them.

Excluded on purpose: editor-owned files (`obj.txt`, `gamma.dat`, `stderr.txt`), relative-only tile names with no `data\global\tiles` prefix, save-file examples, and directory names.
