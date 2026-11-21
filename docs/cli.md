# Command line

`CapybarasD2Editor.exe`, the exe of Capybara's D2 Editor, is also a
command-line tool. When the first argument
is one of the verbs below, the exe runs it and exits without opening a
window. With no arguments, or a first argument that isn't a verb, the editor
starts as usual.

```
CapybarasD2Editor.exe <verb> [options]
```

| Verb | What it does |
|---|---|
| [`render`](#render) | Render a DS1 map to a PNG |
| [`index`](#index) | Show or build a profile's asset index |
| [`maps`](#maps) | List a profile's maps |
| [`profiles`](#profiles) | List the profiles |
| [`help`](#help) | Show help, for all verbs or one |

All verbs only read game data. They write only the `--out` file you name,
files in `capybara\cache\` and, the first time on a clean folder, the
default profiles in `capybara\profiles\` (see below). They read the
editor's `capybara\config.toml` (for map overrides) but never change it.

## General rules

- Verbs, subcommands and flag names are not case-sensitive.
- Every flag takes a value, as `--flag value` or `--flag=value`. A value
  can't start with `--`.
- Giving the same flag twice, or an unknown flag, is a usage error.
- `--help`, `-h` and `/?` as the first argument work like `help`: they
  print the help and exit with 0 instead of starting the editor.
  `--help render` shows the help of one verb, and so does a help flag
  anywhere after a verb (`render --help`, `maps -h`): it prints that verb's
  help and exits with 0.
- Output goes to stdout. Errors go to stderr as `error: ...`, warnings
  as `warning: ...` and notes as `note: ...`.

### Where the CLI looks for profiles

The CLI uses the same `capybara\` folder as the editor: the one next to the
exe, or the folder in the `CAPY_HOME` environment variable when it is set.

`--profile` takes either:

- a profile **id**, read from `capybara\profiles\<id>.toml`; or
- a **path** to a profile TOML file. Anything ending in `.toml`, or
  containing `\` or `/`, is treated as a path.

The profile's `saves` layer points at `capybara\saves\<id>\`, so maps you
saved in the editor are used here too.

### First run without the editor

You don't have to start the editor before using the CLI. When a verb other
than `help` finds no profile in `capybara\profiles\`, it first writes the
same default profiles the editor writes on its first start, and prints a
`note: wrote default profile <file>` line on stderr for each:

- `vanilla.toml` for Diablo II in its default folder,
  `C:\Program Files (x86)\Diablo II`, or else in the folder the Diablo II
  registry key names (`InstallPath`, read only) when that folder holds
  `d2data.mpq` and `d2exp.mpq`;
- `mxl.toml` for Median XL when it is found in an auto-detected folder.

Installs that aren't found get no profile; add your own `.toml` to
`capybara\profiles\` (or use the editor's "Add game install...") for them.
Existing profile files are never changed, and once the folder holds any
profile nothing is written. If the folder can't be written, a warning is
printed and the verb runs anyway.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success, including a render with missing tiles or objects (they are warnings) |
| 1 | Runtime error: a file can't be read or written, a profile won't open, a map's tile set can't be found, the render failed |
| 2 | Usage error: a missing, unknown or repeated flag, or a bad value. The verb's usage is printed after the error. |

Code 2 means the command line itself is wrong. Anything that depends on the
files, such as a `.ds1` outside the profile whose tile set can't be found
without `--lvltype`, is a runtime error with code 1.

### Running from cmd or PowerShell

The release exe is a Windows GUI program, so it has no console window of its
own. When you run a verb from a console, it attaches to that console to print
its output. Because Windows doesn't wait for GUI programs, an interactive cmd
prompt can come back **before the output has finished printing**. The output
then appears after the prompt. The command still ran normally.

Redirection and exit codes work:

```
CapybarasD2Editor.exe maps list --profile vanilla > maps.txt
```

To make an interactive cmd prompt wait and then read the exit code:

```
start "" /wait CapybarasD2Editor.exe profiles list
echo %ERRORLEVEL%
```

Batch files wait for the program by themselves. In PowerShell, piping the
output (for example `| Out-Host`) makes it wait and sets `$LASTEXITCODE`.

Development builds (`cargo build` without `--release`) are console programs
and don't have this quirk.

---

## render

Render one map to a PNG, terrain plus objects and monsters, with the same
rendering library as the editor.

```
render <map> --profile <id|path.toml> [--lvltype N [--def N]] [--zoom 1:N]
       [--gamma G] [--layers LIST] [--variant reference|documented]
       [--object-frame N] --out <file.png>
```

`<map>` is either:

- a **game path** such as `data\global\tiles\act1\town\towne1.ds1`, looked up
  through the profile's layers, including your saves; or
- a **file on disk**. If the file is inside the profile's saves folder, or
  under a folder named `data`, its game path is worked out from its location
  and the tile set is found as for a game path.

| Flag | Default | Meaning |
|---|---|---|
| `--profile` | required | Profile id or TOML path. |
| `--out` | required | PNG file to write. |
| `--lvltype N` | saved override, else best match | Use LvlTypes id N instead of the editor's override or the map catalog's choice. A `.ds1` that isn't part of the profile needs it, unless an override exists for its game path. |
| `--def N` | none | LvlPrest Def to use with `--lvltype` (DS1Edit's command-line form). Needs `--lvltype`. |
| `--zoom 1:N` | `1:1` | Zoom divisor N from 1 to 16. `1:4` and `4` mean the same. |
| `--gamma G` | `1.30` | Palette gamma, above 0 and up to 10. 1.30 is DS1Edit's default. |
| `--layers LIST` | everything except `special` | Comma-separated: `floors`, `walls`, `shadow`, `roofs`, `special`, `objects`, or `all` (everything including `special`). A list without `objects` renders the terrain only. |
| `--variant` | `reference` | How to choose among tile variants: `reference` matches DS1Edit; `documented` takes the first tile with the highest rarity. |
| `--object-frame N` | `0` | Animation frame of every object and monster sprite, 0 to 255. Each sprite wraps N at its own frame count, so a 10-frame torch shows frame `N mod 10`. |

Without `--lvltype`, the tile set is chosen the same way the editor does (see
[How the tile set and palette are chosen](user-guide.md#how-the-tile-set-and-palette-are-chosen)),
including the LvlType/Def **overrides** you set in the editor for this
profile. They are read from `capybara\config.toml` (its `[[overrides]]`
entries whose `profile` is this profile's id). A missing file means no
overrides. If the file can't be read or isn't valid TOML, or an entry is
malformed, a warning is printed and the file (or that entry) is ignored.
An explicit `--lvltype` always wins over an override.

Objects and monsters are drawn while the `objects` layer is on (the
default), in DS1Edit's draw order, with their palette shifts and default
facing. DS1Edit shows animated sprites at a random frame; the CLI shows
frame `--object-frame` for all of them.

### Output

The first line describes the map context:

```
map <map>: source=<Source> lvltype=<N> def=<N> act=<N> palette=act<N> dt1s=<N> coverage=<C>
```

The last line reports the render:

```
rendered 1:<N> gamma <G>: <W>x<H> px, origin <X>,<Y>, missing tiles: <N>, objects: <D> drawn, <M> missing, wrote <file>
```

When `--layers` leaves out `objects`, the objects part reads `objects: off`.

Missing tiles and missing objects don't make the command fail. It still
exits with 0, and prints warnings:

- the most common missing tiles, as
  `<layer>:o<orientation>/m<main>/s<sub> x<count>`;
- the objects that draw nothing, as `type <T> id <I> (<reason>) x<count>`.
  The reasons are `unresolved`, `no sprite token`, `COF not found: <path>`,
  `COF unusable: <path>` and `no component sprite found for <path>`;
- objects placed outside the map's cells. They are skipped as in DS1Edit
  and count as neither drawn nor missing.

Missing DT1 files and the object tables' warnings are printed too. For
example, vanilla has no ObjPreset table, so DS1Edit's `obj.txt` is used and
a warning says so.

If the map isn't part of the profile and its tile set can't be found (no
`--lvltype` and no override), the command stops with exit code 1 and asks
for `--lvltype`.

### Examples

Render a vanilla town map at 1:2:

```
CapybarasD2Editor.exe render data\global\tiles\act1\town\towne1.ds1 --profile vanilla --zoom 1:2 --out towne1.png
```

Render a map with every animated object at its third frame:

```
CapybarasD2Editor.exe render data\global\tiles\act2\town\lutw.ds1 --profile vanilla --zoom 1:4 --object-frame 2 --out lutw.png
```

Render a map file from disk with an explicit LvlType and Def, floors and
walls only (no objects):

```
CapybarasD2Editor.exe render C:\work\mymap.ds1 --profile mxl --lvltype 2 --def 40 --layers floors,walls --out mymap.png
```

---

## index

Show or build a profile's asset index (`capybara\cache\index.db`). It is the
same index the editor builds in the background.

```
index status --profile <id|path.toml>
index build  --profile <id|path.toml>
```

- `status` prints one line: `missing`, `up to date`, or `stale: <reason>`.
- `build` rebuilds the index, printing progress lines at most once a second
  (and at every phase change):

  ```
  [tiles] 120/655 data\global\tiles\act1\town\floor.dt1
  ```

  Phases are `listing`, `tables`, `maps`, `tiles`, `sprites`, `thumbnails`
  and `writing`. The last line is a summary:

  ```
  built index for <id>: <F> files, <A> assets, <T> tiles, <W> warnings in <S>s
  ```

  Up to five warnings are printed, followed by a count of the rest.

### Example

```
CapybarasD2Editor.exe index status --profile mxl
CapybarasD2Editor.exe index build --profile mxl
```

---

## maps

List a profile's maps, with how each one's tile set is found.

```
maps list --profile <id|path.toml> [--filter S]
```

Each line has a game path, a tab, and the map catalog source of that map's
LvlType: `Override`, `LvlPrest`, `PathGuess`, `TileCoverage` or `Ds1Header`
(see [the user guide](user-guide.md#how-the-tile-set-and-palette-are-chosen)
for what each means). `Override` marks a map whose LvlType you set in the
editor. The overrides are read from `capybara\config.toml`, as for
[`render`](#render). `--filter` keeps paths that contain `S`, ignoring
case. If nothing matches, nothing is printed.

The list includes maps in your saves folder. The first run for a profile
gathers file names from the archives and caches them in
`capybara\cache\known-names-<id>.txt`, shared with the editor. Later runs
are faster.

### Example

```
CapybarasD2Editor.exe maps list --profile vanilla --filter town
```

---

## profiles

List the profiles in `capybara\profiles\`.

```
profiles list
```

Each line is the id, the game version and the name, separated by tabs, for
example:

```
vanilla	1.14d	Diablo II 1.14d
```

The version is printed as in the profile file: `1.13c`, `1.14d` or `d2r`.
On a clean folder the default profiles are written first (see
[First run without the editor](#first-run-without-the-editor)). With no
profiles and no install found it prints `no profiles in <folder>`. A
profile file that can't be read is an error (exit code 1).

---

## mcp

```
mcp --profile <id|path.toml>
```

Serves the Model Context Protocol over stdin/stdout (JSON-RPC 2.0, one
message per line) so an AI assistant can edit maps with the editor's own
logic. Nothing but protocol messages is written to stdout. Register it in an
MCP client as the command `CapybarasD2Editor.exe mcp --profile vanilla`.

Tools: `list_maps`, `open_map`, `new_map`, `map_info`, `get_cells`, `paint`,
`set_cell`, `copy_area`, `find_replace`, `reroll`, `resize_map`,
`set_cell_flags`, `list_objects`, `add_object`, `edit_objects`, `undo`,
`redo`, `check_map`, `render_map` (returns a PNG), `save_map`, `regions`,
`no_spawn_area`, `list_tiles`, `list_placeable`, `add_layer`, `clear`,
`history`, and the tile-creation tools below.

One map is open at a time; edits can be undone until the map is replaced.
`save_map` writes to `capybara\saves\<profile>\<game path>`, adds the file to
`CHANGED.txt` and records a history snapshot, exactly as the editor's Save
does. The game folder is never written. A tool that fails returns a result
marked `isError` with the reason.

### Creating tiles

New DT1 tiles are built the way shipped tiles of their kind are (see
`docs/specs/dt1.md` section 14). A **shape** is given as:

| argument | meaning |
|---|---|
| `orientation` | 0 floor, 1-9, 12, 14 upper walls (1 left, 2 right, 3/4 the halves of a north corner, 7 south corner, 8/9 doors, 12 pillar, 14 tree), 10/11 special, 13 shadow, 15 roof, 16-19 lower walls |
| `height` | walls, shadows, specials, lower walls: the picture height in pixels, 1-2016, rounded up to a multiple of 32 |
| `roof_height` | roofs: how far up the game lifts the roof (default 80) |
| `corner_pair` | with orientation 3 or 4: one 160-wide picture becomes both halves (x 0-79 orientation 4, x 80-159 orientation 3), numbered together |

Tools:

- `tile_template {shape, mask?}`: the picture size for the shape, where its
  pixel (0,0) is in tile space, what part is drawable (floors and roofs: the
  160x79 diamond only), the sub-block count and the default sub-tile flags.
  `mask: "base64"` adds the drawable area as one byte per pixel (1 =
  drawable), `mask: "png"` as a PNG.
- `create_tile {shape, pixels?, fill?, placement?, transparent_index?,
  target...}`: `pixels` is `{data, width, height}`: `data` is base64 of
  `width * height` palette indices, row by row, in the picture of
  `tile_template` (pixel (0,0) on its top-left; at most 2048 a side). Index 0
  (or `transparent_index`) is empty. `fill` (1-255) paints the whole drawable
  area first; with neither the tile is blank. `placement`: `as_is` (default),
  `center` or `fit` (scaled to fit, aspect kept).
- `import_tile_image {shape, path | data, format?, placement?, dither?,
  transparent_rgb?, alpha_threshold?, act?, target...}`: a PNG, BMP, GIF
  (first frame) or PCX picture, from a file (at most 16 MB) or base64 `data`,
  at most 4096 pixels a side; placed, then quantized to the act palette (the
  map's by default). `dither`: `none` or `floyd_steinberg`; `transparent_rgb`
  `[r, g, b]` (default black) or `null` for alpha only.
- `reshape_tile {dt1?, index | orientation+main+sub(+variant), new_orientation?,
  new_height?, new_roof_height?}`: re-splits a tile of an own DT1 into another
  shape; its pixels stay where they are and what no longer fits is reported.
- `new_dt1 {name, folder?, register?}`: an empty DT1
  `<folder>/<name>.dt1` (letters, digits and `_`, within the game's path
  length) in the profile's saves; `folder` defaults to the open map's own-DT1
  folder. `register: true` also registers it for the open map's level type
  (LvlTypes File slot and LvlPrest Dt1Mask) and lists it in the map.
- `list_own_tiles {dt1?, limit?}`: the tiles of the open map's own DT1 (or
  `dt1`): number, kind, shape, rarity, sound, blocked sub-tiles, blocks.
- `inspect_tile {dt1?, index | orientation+main+sub(+variant), render?, name?,
  act?}`: header fields, sub-tile flags, blocks, and a picture: `render:
  "image"` (default, a PNG in the result), `"file"` (written to
  `capybara\renders\<name>.png`) or `"none"`.

**Target** (`create_tile`, `import_tile_image`): without `dt1` the tile goes
into the open map's own DT1 (`<level type>_custom.dt1`, or
`<map>_custom.dt1`), exactly like the editor's *Add to this map*: its number
avoids every DT1 of the level type (a clash is renumbered; corner halves stay
together), the DT1 is registered in LvlTypes and the map's LvlPrest Dt1Mask
(on 1.13c the compiled `.bin` fields are patched in place and the `.txt`
forms kept in step) and the map's DS1 lists it (one undo step; `save_map`
keeps it). With `dt1` the tile is appended to that DT1, which must already be
in the profile's saves (made by `new_dt1` or an earlier addition); DT1s of the
game are never changed. `main` (0-63) and `sub` (0-255) ask for a number;
`flags` (25 numbers, `y * 5 + x`), `rarity` and `sound` set the header. The
result names the number each tile got, lists every file written and warns
about pixels outside the drawable area and opaque pixels of palette index 0
(which DT1s draw as transparent). Special tiles (10, 11) can only go into a
`dt1`, as in the editor.

After a tile is added the map's tile set is reloaded, so `list_tiles`,
`paint`, `render_map` and `check_map` use the new tile at once. All files go
to `capybara\saves\<profile>` with `CHANGED.txt` and a history snapshot.

---

## help

```
help [verb]
--help [verb]
-h [verb]
/? [verb]
```

Without a verb, prints an overview of all verbs and the exit codes. With a
verb (`render`, `index`, `maps`, `profiles` or `help`), prints that verb's
usage. The four spellings are the same command, and exit with 0. An unknown
verb is a usage error.
