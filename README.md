# Capybara Map Editor

A portable, single-exe Windows editor for Diablo II `.ds1` maps and DC6/DCC
sprites. It reads the game's MPQs read-only, never writes to the game folder,
and keeps everything it writes in a `capybara\` folder next to the exe.

<!-- screenshot: the Map Editor with a town map open, the Maps list on the left and Layers/Properties on the right -->

## Features

- **Map editing.** Textured map view that draws tiles in DS1Edit's order. Box,
  lasso, tile-ID and "select identical" selection, paint, erase, eyedropper,
  and object move/delete. Copy and paste across maps with a conflict preview.
  You can move a selection between layers (for example Wall 1 to Wall 3), and
  the editor adds the target layer if the map lacks it. Also flags, resize,
  layer actions, and unlimited undo/redo for each map.
- **Sprites mode.** A DC6/DCC/COF viewer with palettes, gamma, COF layers and
  playback. A converter that imports and exports PNG, BMP, GIF and PCX,
  including batch conversion of whole folders. A palette-locked sprite editor
  with gradient, shading, outline, selections, frames, onion skin and a DCC
  cell check.
- **Asset index.** A background index of every tile and sprite in the profile,
  with a progress bar in the status bar. The tile browser and Ctrl+K search
  that use it are in development.
- **History.** Every save records a deduplicated snapshot in
  `capybara\history\`. The timeline and restore UI are planned.
- **Median XL support.** Reads Median XL's split `medianxl-*.mpq` archives on
  top of its own base MPQs, and its compiled `.bin` tables (no `.txt` needed).
- **Command line.** The same exe renders maps to PNG and lists maps and
  profiles without opening a window. See [docs/cli.md](docs/cli.md).

## Portable layout

```
CapybaraMapEditor.exe
capybara\
  config.toml          settings, recent maps, tile-set overrides
  layout.json          Map Editor panel layout
  layout-sprites.json  Sprites panel layout
  profiles\<id>.toml   one game profile per file
  saves\<profile>\     your edited files, mirroring game paths, plus CHANGED.txt
  history\<profile>\   snapshots of every save
  cache\               asset index and name lists (safe to delete; rebuilt automatically)
```

Copy or zip the folder to move your whole setup.

## Quick start

1. Put `CapybaraMapEditor.exe` in a folder you can write to, for example
   `C:\Tools\Capybara\`. Don't use `C:\Program Files\`: the editor must be
   able to create `capybara\` next to the exe, and it shows an error and
   stops if it can't.
2. Start it. On the first run it creates `capybara\` and writes a profile for
   each install it finds at the default locations:
   - `C:\Program Files (x86)\Diablo II` becomes the `vanilla` profile;
   - `C:\games\median-xl` becomes the `mxl` profile.

   If your game is somewhere else, copy a profile file into
   `capybara\profiles\` and edit the paths (see the
   [user guide](docs/user-guide.md#game-profiles)).
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

The full manual is [docs/user-guide.md](docs/user-guide.md).

## Building from source

You need Rust 1.99 or newer (stable, `x86_64-pc-windows-msvc`) and the MSVC
build tools.

```
cargo build --release -p capy-app
```

The exe is `target\release\CapybaraMapEditor.exe`. For development, use the
faster-linking dev feature (never for release builds):

```
cargo run -p capy-app -F dev
```

## Credits

Capybara Map Editor owes its understanding of the file formats to Paul
Siramy's win_ds1edit and Ben Ethington's DS1Edit. See [NOTICE](NOTICE) for
full credits and third-party licenses. Diablo II is the property of Blizzard
Entertainment, and Median XL belongs to its authors. Neither game is
distributed with this program.

## License

MIT. See [LICENSE](LICENSE).

Third-party licenses for everything bundled in the exe: [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
