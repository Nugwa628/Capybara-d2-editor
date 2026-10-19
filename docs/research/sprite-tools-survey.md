# DC6/DCC/COF/.sprite tools: survey and gaps (2026-10-19)

Public web pages only; nothing downloaded or run. Several Phrozen Keep,
Nexus and DeepWiki pages timed out or returned 403/429; those entries rest
on search snippets and are marked unverified.

## Tools found
| Tool | Author | Platform / status | License | Notable features |
|---|---|---|---|---|
| pd2-sprite-studio | RoofooEvazan | Windows (Electron), new, active | not stated | Palette-locked pixel editor; selections incl. mirror to opposite direction; full COF compositing with ghosted layers, onion skin, live AnimData preview; armtype/weapon/body swaps; palette ramp tools, ΔE picker; outline/clean-up tools; 3D model/render import with 8↔16 direction mapping; GLB/FBX → DT1 + DS1 tile maker; DCC 4-colour-per-cell check; GIF/sheet/DC6/DCC/COF export; item tint preview. https://github.com/RoofooEvazan/pd2-sprite-studio |
| SixDice v0.63 | Zachary E. Palmer | Java, abandoned | personal/non-profit, no reverse engineering | DC6/DCC read/write, batch, split/join, trim, center, underlay, offset metadata. https://bahj.com/sixdice/ |
| HellSpawner | OpenDiablo2 (fork: gucio321) | Go, upstream stalled 2021 | GPL-3.0 | DC6/DCC view; COF, TBL, TXT, DAT, PL2, DT1, DS1, animdata editors; MPQ browse. https://github.com/OpenDiablo2/HellSpawner |
| gravestench/dcc | gravestench | Go lib + CLI, active | not stated | dcc-info (JSON), dcc-check, dcc-convert (PNG, .gpl palette), byte-identical lossless re-encode of unmodified files (no new-pixel compressor). https://github.com/gravestench/dcc |
| MergeDCC / DII Animation Extractor | Paul Siramy | DOS/Win, old | source available | Flatten COF layers into one-layer frames at uniform canvas, colormap pick, mirroring on export. https://www.allegro.cc/depot/MergeDCC |
| DC6Color | Paul Siramy | Win, 2007 | source included | Sheet of all 126 Unique/Set/Socketed colormap variants. https://www.allegro.cc/depot/DC6Color |
| TileMaker2 | Paul Siramy | CLI, old | not stated | Image → diamond tiles. |
| Worldstone / OpenDiablo2 pl2 | Lectem / OpenDiablo2 | C++ / Go | GPL-3.0 / n.s. | PL2 palette-shift generation matching the game. |
| AnimData_Edit, d2animdata, TXTeditor | Siramy / PyPI / Nexus | old / alpha / ? | ? | AnimData.D2 edit; TSV/JSON round trip; validation. |
| D2RModding-SpriteEdit, D2RSpriteConverter, D2R Sprite ED2R | eezstreet / AtsusaKai / KR community | .NET-ish | GPL-3.0 / n.s. / ? | D2R `.sprite` read/write (+ `.lowend` generation), batch convert, GIF export (formats unverified). |
| DC6 Creator, dc6con, lazarus, dc6png, CvDcc, COF-Edit, Coffee … | various | old | ? | Mostly DC6⇄PCX/PNG conversion; many names only (unverified). |
| bethington/d2-ds1-edit | Ben Ethington | active | MIT additions | Object DCC/DC6 render with shadow projection, trans_b blends. (Our reference.) |

## Gaps vs our baseline (viewer + SixDice-class converter)
- **Authoring metadata:** COF editing (M); AnimData editing with TSV/JSON export (S–M); palette .dat/PL2 editing & PL2 regeneration (M–L); palette helpers (S); .gpl/.act/.pal palette export (S).
- **Preview:** item colormap / palshift / transform variants (S–M); onion skin, layer ghosting, reference overlay, diff (S–M); light-level preview (S); object shadow projection in map render (S–M); DCC/DC6 inspect/validate CLI incl. DCC 4-colour-cell check (S); transparency underlay (S).
- **Editing:** palette-locked pixel editor (L); ramp shift/shade/outline tools (M, needs pixel editor); edit transfer across frames (L); mirror direction (S–M); 8↔16 direction remap (S–M); layer/body swap (M).
- **Export:** flatten COF to one-layer animation (S); animated GIF (S); byte-identical re-encode guarantee for unmodified DCC (S as test); render-folder import with facing map (M); 3D scene → sprite (L); image/3D → DT1 tiles (L; M for image splitting); indexed PNG with embedded palette (S).
- **D2R:** `.sprite` read/write + `.lowend` (M, header needs verification); tall-sprite split, batch resize (S).
- **Meta:** MCP server (S, low value; capy-cli exists).

## Surveyor's top 5
1. D2R `.sprite` read/write + low-end generation (prerequisite for useful CASC).
2. COF + AnimData editing with validation (no maintained tool exists).
3. Colormap / palshift / PL2 light previews incl. item variants.
4. Onion skin, layer ghosting, mirror/direction-remap export.
5. Flatten layers to one-layer animation + animated GIF export.
