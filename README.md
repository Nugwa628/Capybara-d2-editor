# Capybara's D2 Editor

A portable, single-exe Windows editor for Diablo II `.ds1` maps and DC6/DCC
sprites. It reads the game's MPQs read-only, never writes to the game folder,
and keeps everything it writes in a `capybara\` folder next to the exe.

The map editor's interface, layout and many of its map-editing features follow
[**DS1 Studio**](https://github.com/RoofooEvazan/ds1-studio) by RoofooEvazan
(fork: [D2R-Gimli/ds1-studio](https://github.com/D2R-Gimli/ds1-studio)). All
credit for that design goes to its author; go check it out.

<!-- screenshot: the Map Editor with a town map open, the Maps list on the left and Layers/Properties on the right -->

## System requirements

- Windows 10 or 11, 64-bit.
- A graphics driver with Vulkan or DirectX 12 support.
- A Diablo II install (classic 1.13c/1.14d, or Median XL). It is only read.

## Features
![Uploading image.png…]()

- **Map editing.** Textured map view that draws tiles in DS1Edit's order. Box,
  lasso, tile-ID and "select identical" selection, paint, erase, eyedropper,
  and object move/delete. Copy and paste across maps with a conflict preview.
  You can move a selection between layers (for example Wall 1 to Wall 3), and
  the editor adds the target layer if the map lacks it. Also flags, resize,
  layer actions, and unlimited undo/redo for each map.
- **Objects and NPCs.** Objects and NPCs drawn and animated on the map,
  placement, properties, NPC path editing, and import of DS1Edit's `.ini`
  mappings.
- **Sprites mode.** A DC6/DCC/COF viewer with palettes, gamma, COF layers and
  playback. A converter that imports and exports PNG, BMP, GIF and PCX,
  including batch conversion of whole folders. A palette-locked sprite editor
  with gradient, shading, outline, selections, frames, onion skin and a DCC
  cell check.
- **Palette tools.** Convert tiles and sprites (DT1, DC6, DCC) to another
  act's or UI palette with an adjustable colour plan, overrides, before/after
  preview and batch, and a per-file palette memory that warns before art is
  saved under the wrong palette.
- **Tile browser and search.** A background index of every tile and sprite in
  the profile, a Tiles browser with thumbnails, favorites, recents and saved
  searches, and Ctrl+K search with filters such as `type:floor color:green`.
- **History.** Autosave, named checkpoints, a timeline with a visual diff and
  restore, retention limits and optional git milestones, all in
  `capybara\history\`.
- **Median XL support.** Reads Median XL's split `medianxl-*.mpq` archives on
  top of its own base MPQs, and its compiled `.bin` tables (no `.txt` needed).
- **Command line.** The same exe renders maps to PNG and lists maps and
  profiles without opening a window. It works before the window has ever been
  opened, because it writes the default profiles itself. See
  [docs/cli.md](docs/cli.md).
- **Loading screen.** Diablo II's door-opening animation with a capybara in
  the door while the game data loads. Click or Esc skips it, and Settings can
  turn it off.

## Portable layout

```
CapybarasD2Editor.exe
capybara\
  config.toml          settings, recent maps, tile-set overrides
  layout.json          Map Editor panel layout
  layout-sprites.json  Sprites panel layout
  profiles\<id>.toml   one game profile per file
  saves\<profile>\     your edited files, mirroring game paths, plus CHANGED.txt
  history\<profile>\   snapshots of every save
  cache\               asset index and name lists (safe to delete; rebuilt automatically)
  crash.log            details of a crash, if one ever happens
```

Copy or zip the folder to move your whole setup.

## Quick start

1. Put `CapybarasD2Editor.exe` in a folder you can write to, for example
   `C:\Tools\Capybara\`. Don't use `C:\Program Files\`: the editor must be
   able to create `capybara\` next to the exe, and it shows an error and
   stops if it can't.
2. Start it. On the first run it creates `capybara\` and writes a profile for
   each install it finds: Diablo II in `C:\Program Files (x86)\Diablo II` or
   wherever its installer recorded it becomes the `vanilla` profile, and a
   Median XL install it recognises becomes the `mxl` profile.

   If your game is somewhere else, use **File > Add game install...** and
   pick its folder (see the [user guide](docs/user-guide.md#game-profiles)).
3. Pick a profile in the **Profile** box on the menu bar. Opening a profile
   takes a few seconds; progress shows in the status bar.
4. Click a map in the **Maps** panel. Type in **Filter** to narrow the list,
   for example `town`.
5. Edit, then press **Ctrl+S**. The map is written to
   `capybara\saves\<profile>\data\global\tiles\...` and its game path is added
   to `capybara\saves\<profile>\CHANGED.txt`.
6. Pack the changes yourself. The editor never writes MPQs. Use
   **File > Open saves folder**, then drag the `data` folder into your MPQ
   editor: the paths already match. `CHANGED.txt` lists exactly the files
   to pack.
7. After packing, use **File > Mark as packed** to empty `CHANGED.txt`.

Maps you have saved open from `capybara\saves\` from then on, because saves
override the game archives.

If the editor ever stops with an error message, the details are in
`capybara\crash.log`; please include that file in a bug report.

The full manual is [docs/user-guide.md](docs/user-guide.md).

## Building from source

You need Rust 1.99 or newer (stable, `x86_64-pc-windows-msvc`) and the MSVC
build tools with the Windows SDK.

```
cargo build --release -p capy-app
```

The exe is `target\release\CapybarasD2Editor.exe`; `scripts\build-release.ps1`
builds, checks and zips a release. The build embeds the icon and version
information with `rc.exe` from the Windows SDK; without it the build still
succeeds and prints a warning. For development, use the faster-linking dev
feature (never for release builds):

```
cargo run -p capy-app -F dev
```

## Credits

Capybara's D2 Editor owes its understanding of the file formats to Paul
Siramy's win_ds1edit and Ben Ethington's DS1Edit. The map editor's interface,
layout, themes, shortcuts and editing tools follow RoofooEvazan's
[DS1 Studio](https://github.com/RoofooEvazan/ds1-studio). Its icon and loading screen
come from the author's earlier MXL Capybara Editor. See [NOTICE](NOTICE) for
full credits and third-party licenses. Diablo II is the property of Blizzard
Entertainment, and Median XL belongs to its authors. Neither game is
distributed with this program; the loading screen's door animation and font
are read from your own game files.

## License

MIT. See [LICENSE](LICENSE).

Third-party licenses for everything bundled in the exe: [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
