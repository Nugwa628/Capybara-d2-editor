# Capybara's D2 Editor user guide

This guide is for Diablo II map modders. It covers editing `.ds1` maps, saving
them for packing, and working with DC6/DCC sprites. The command line is
covered in [cli.md](cli.md); it works before the editor's window has ever
been opened, because it writes the default profiles itself.

- [Getting started](#getting-started)
- [The window](#the-window)
- [Opening maps](#opening-maps)
- [Viewing a map](#viewing-a-map)
- [Editing a map](#editing-a-map)
- [Saving and packing](#saving-and-packing)
- [History: autosave, checkpoints, diff and restore](#history-autosave-checkpoints-diff-and-restore)
- [Settings](#settings)
- [Sprites mode](#sprites-mode)
- [The Tiles browser and Ctrl+K search](#the-tiles-browser-and-ctrlk-search)
- [Troubleshooting](#troubleshooting)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Advanced: settings and environment variables](#advanced-settings-and-environment-variables)

---

## Getting started

### System requirements

- Windows 10 or 11, 64-bit.
- A graphics card whose driver supports Vulkan or DirectX 12 (any card and
  driver from the last several years). Without one the editor cannot open its
  window; it says so in a message box and in `capybara\crash.log`.
- A Diablo II install (classic 1.13c/1.14d, or a mod such as Median XL). The
  editor only reads it.

### The portable `capybara\` folder

The editor is one file, `CapybarasD2Editor.exe`. Everything it writes goes
into a folder called `capybara\` next to the exe. It never writes to your
game install.

```
CapybarasD2Editor.exe
capybara\
  config.toml          settings, recent maps, your tile-set overrides
  layout.json          panel layout of the Map Editor
  layout-sprites.json  panel layout of Sprites mode
  profiles\<id>.toml   one game profile per file
  saves\<profile>\     your edited files, at their game paths, plus CHANGED.txt
  history\<profile>\   snapshots of saves, autosaves and checkpoints (index.json and objects\)
  cache\               asset index (index.db) and file-name lists
  crash.log            written only if the editor crashes (see Troubleshooting)
```

- `cache\` is safe to delete. The editor rebuilds it in the background.
- `saves\` and `history\` hold your work. Back them up.
- To move your whole setup, copy or zip the exe together with `capybara\`.

The folder must be writable. If the editor can't create or write to it, it
shows an error message and stops (see [Troubleshooting](#troubleshooting)).

### The loading screen

While the editor opens its game data, it shows Diablo II's own door-opening
loading animation, taken from the start profile's game files, with a capybara
standing in the door and "Capybara's D2 Editor" on the plate. It stays until
the profile's map list is ready (at least one pass of the animation), then
fades out. Click or press **Esc** to skip it. Any question the editor asks at
start-up (for example **Add game install**) closes it first, so nothing hides
behind it. Without game files it shows the capybara and the name on a dark
background.

Turn it off in **File > Settings... > Start-up > Show loading screen**.

### Game profiles

A profile tells the editor which archives and folders make up one game setup.
Each profile is a TOML file in `capybara\profiles\`.

On the first run, the editor writes a profile for each install it finds:

| Profile id | Name | Where it looks | Game version |
|---|---|---|---|
| `vanilla` | Diablo II 1.14d | `C:\Program Files (x86)\Diablo II`, then the install path Diablo II's installer left in the Windows registry | 1.14d |
| `mxl` | Median XL | a Median XL folder the editor recognises (it has no fixed install location, so use **Add game install...** if none is found) | 1.13c |

The registry is only read, never written. It is checked under
`HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Blizzard Entertainment\Diablo II` and
`HKEY_CURRENT_USER\Software\Blizzard Entertainment\Diablo II` (value
`InstallPath`); a registry path is used only if it holds `d2data.mpq` and
`d2exp.mpq`.

It only does this when `config.toml` doesn't exist yet or `capybara\profiles\`
holds no `.toml` files. It never overwrites a profile file.

If your game lives somewhere else, use **File > Add game install...** (see
below), write the profile yourself, or copy a generated one and change the
paths. Profile files you write by hand are read when the editor starts, so
restart it after adding or editing one.

Switch profiles with the **Profile** box on the menu bar. At start-up the
editor opens the profile you used last. On the very first run it opens
`vanilla` if there is one, otherwise the first profile in alphabetical order
of id.

#### Add game install

**File > Add game install...** (also in **File > Settings...**) makes a
profile for an install folder:

1. Click **Choose folder...**, type the folder, or click **Use** next to an
   install the editor found (from the registry or a default location).
2. The editor looks at the archive names in the folder:
   - any `medianxl-*.mpq`: **Median XL**;
   - `d2data.mpq` and `d2exp.mpq`: **Diablo II (vanilla)**;
   - neither: it asks you which one to treat it as.
   If another profile already reads that folder, the dialog says so.
3. Check the proposed **Profile id** (unique, for example `vanilla-2`),
   **Name** and **Game version**.
4. Click **Add profile**. The editor writes `capybara\profiles\<id>.toml` and
   opens the new profile straight away; no restart is needed.

Nothing is ever written inside the install folder.

#### Vanilla

The `vanilla` profile reads, highest priority first: your saves, then
`Patch_D2.mpq`, `d2exp.mpq`, `d2data.mpq` and `d2char.mpq` from the vanilla
install.

#### Median XL

The `mxl` profile reads: your saves, then every `medianxl-*.mpq` in the
Median XL folder, then Median XL's **own** copies of `patch_d2.mpq`,
`d2exp.mpq`, `d2data.mpq` and `d2char.mpq`. It never uses the vanilla
install's archives, because Median XL ships modified base MPQs. Median XL
ships its data tables only as compiled `.bin` files, and the editor reads
those directly.

#### Adding your own profile

Create `capybara\profiles\<id>.toml`. This example puts a mod's loose folder
and MPQ on top of a vanilla 1.13c install:

```toml
id = "mymod"
name = "My mod"
game_version = "1.13c"

[[layer]]
kind = "saves"

[[layer]]
kind = "folder"
path = 'C:\mods\mymod\extracted'

[[layer]]
kind = "mpq"
path = 'C:\mods\mymod\mymod.mpq'
optional = true

[[layer]]
kind = "mpq"
path = 'C:\Games\Diablo II 1.13c\Patch_D2.mpq'
optional = false

[[layer]]
kind = "mpq"
path = 'C:\Games\Diablo II 1.13c\d2exp.mpq'
optional = false

[[layer]]
kind = "mpq"
path = 'C:\Games\Diablo II 1.13c\d2data.mpq'
optional = false

[[layer]]
kind = "mpq"
path = 'C:\Games\Diablo II 1.13c\d2char.mpq'
optional = false
```

The generated `vanilla.toml` and `mxl.toml` use the same format. Read them
for more examples.

| Key | Meaning |
|---|---|
| `id` | Lowercase letters, digits, `_` and `-` only. It also names the profile's folders in `saves\` and `history\`, so don't change it once you have saved work. |
| `name` | What the Profile box shows. |
| `game_version` | `"1.13c"`, `"1.14d"` or `"d2r"`. It decides how the compiled `.bin` tables are read. |
| `[[layer]]` | One entry per source, **highest priority first**. |

Layer kinds:

| `kind` | Keys | What it reads |
|---|---|---|
| `saves` | none | `capybara\saves\<id>\`. Keep it first so your edits win. |
| `folder` | `path` | Loose files in a folder (an extracted mod). |
| `mpq` | `path`, `optional` | One MPQ archive. With `optional = true` a missing file is skipped; otherwise a missing file stops the profile from opening. |
| `mpq-glob` | `dir`, `pattern` | Every file in `dir` matching `pattern` (`*` and `?`, case-insensitive), in name order. No match is an error. |
| `casc` | `path` | D2R storage. **Not implemented yet.** For D2R, point a `folder` layer at extracted files instead. |

In TOML, single-quoted strings (`'C:\Games'`) keep backslashes as they are.
In double-quoted strings, write each backslash twice (`"C:\\Games"`).

### Load order: saves override archives

When the editor looks up a file, it checks the layers in order and uses the
first one that has it. Because `saves` comes first, a map you've saved opens
as your edited version, not the one in the MPQ. The Maps list colours these
maps green and marks them `saves`.

---

## The window

<!-- screenshot: the whole window, with the mode tabs, menu bar, dock panels and status bar labelled -->

From top to bottom:

1. **Mode tabs**: **Map Editor** and **Sprites**. Each mode has its own
   panels and its own saved layout. Sprites mode works without a map open.
2. **Menu bar**: **File**, **Edit** and **View** menus, and the **Profile**
   box.
3. **Dock panels.** Drag a panel's tab to move it, or drop it on an edge to
   split the area. The layout is saved automatically.
   **View > Reset panel layout** restores the default for the current mode.
4. **Status bar**:
   - the open profile and how many maps it has;
   - a spinner while the map catalog builds;
   - asset index progress, with a **Cancel** button;
   - on the right, the time of the last autosave (`autosaved 14:32`), the
     hovered cell and the zoom (`1:4`).

Map Editor panels (default layout):

| Panel | Where | What it does |
|---|---|---|
| Maps | left | Every map in the profile, with a filter. |
| Tiles | left, second tab | The asset browser: search every tile (and sprite) of the profile, Recent tiles, Favorites and saved searches. See [The Tiles browser](#the-tiles-browser-and-ctrlk-search). |
| Map | centre | Open maps as tabs, the tool strip, and the map itself. |
| Layers | right | Choose the active layer, toggle visibility, layer actions. |
| Objects | right, second tab | The selected object's properties and NPC path, and the **Add object** picker. See [Objects and NPC paths](#objects-and-npc-paths). |
| Properties | right, third tab | Facts about the map, its tile set (with override), rendering status, hovered cell. |
| History | right, fourth tab | The map's snapshots, checkpoints, the diff against a snapshot, restore, milestones. **View > History** shows or hides it. |
| Log | bottom | Everything the editor reports. |
| Sprite preview | optional | The selected object's animation. Turn it on with **View > Sprite preview**. |

Only the Sprite preview, History and Objects panels can be closed. The others
can be moved but not closed. A layout saved by an older version may lack the
History or Objects tab: **View > History** and **View > Objects panel** add
them, and picking the Objects tool (**O**) brings the Objects panel up too.

### The Log and notifications

The **Log** panel lists every info line, warning and error, newest at the
bottom. **Clear** empties it. Sprites mode has its own Log tab showing the
same entries.

Errors and some confirmations also pop up as notifications in the
bottom-right corner. They disappear after about 7 seconds, or click **x**.

---

## Opening maps

### The Maps list

The Maps panel lists every `.ds1` the profile knows about. Paths are shown
without the leading `data\global\tiles\`; hover a row for the full path and
the archive it comes from.

Rows are coloured by where the file comes from:

| Colour | Label | Source |
|---|---|---|
| green | `saves` | your saved copy in `capybara\saves\<profile>\` |
| orange | `mod` | a mod archive or folder (for example `medianxl-*.mpq`) |
| blue | `base` | Diablo II's own MPQs |

Type in **Filter** to keep only paths containing that text. The filter
ignores case, and backslashes work, for example `act1\town`.

Click a row to open the map as a tab in the Map panel. Clicking a map that is
already open switches to its tab. **File > Open recent** lists the last 20
maps you opened in the current profile.

### How the tile set and palette are chosen

A `.ds1` file doesn't say which tile files (DT1s) it uses. The game decides
that from its level tables. When a profile opens, the editor builds a **map
catalog** in the background (the status bar shows "Map catalog..."). The
catalog works out each map's LvlType, LvlPrest Def, act, palette and DT1
list.

The **Properties** panel shows the result under **Map context**. The
**Context** row says where the answer came from:

| Context | Meaning |
|---|---|
| `LvlPrest` | A LvlPrest row names this file (in File1 to File6). Its Level gives the LvlType. This is how the game loads it. |
| `guess (same folder)` | No LvlPrest row names it. The editor picked the LvlType whose tile files sit in the same folder as the map. |
| `guess (tile coverage)` | No LvlPrest row and no folder match. The editor picked the LvlType whose tiles cover the most of the map's cells, but only if they cover at least 90%. |
| `DS1 header only` | Nothing matched. The act comes from the file header and there are no tile files, so every tile shows as missing. |
| `override` | You chose the LvlType yourself (see below). |

When several LvlTypes could fit, they are ranked by tile coverage, then by
how many tile files they share with the map's own list, then by folder match,
then by lowest id.

The same section also shows the Level (id and name), the act, the palette
act, the coverage for guessed tile sets, and how many DT1 files are used. If
some DT1s named by the tables aren't in the profile, an orange line says how
many; hover it for the list.

The palette comes from the Levels table's `Pal` column when the Level is
known. Otherwise it is the LvlType's act, except LvlType 35 (`act5_lava`),
which uses the Act 4 palette, as DS1Edit does.

### Overriding the LvlType and Def

If a map draws with the wrong tiles, choose its tile set yourself in
**Properties > Map context**:

1. Open the **Interpretation** list. It shows every way the catalog can read
   this map, best first. Pick one to use it.
2. Or choose any **LvlType** id from the list, type a LvlPrest **Def**
   number (leave it empty for none), and click **Apply**.
3. To go back to the catalog's own choice, click **Clear override**.

Overrides apply to every open copy of the map. They are saved per profile in
`config.toml`, so the map opens the same way next time.

### DS1Edit's legacy `.ini` mappings

DS1Edit keeps LvlType/Def choices in `assets\*.ini` files, one
`<LvlTypesId> <LvlPrestDef> <path>` per line, for example
`1 2 assets/tiles/ACT1/TOWN/townE1.ds1`. To bring them over:

1. **File > Import DS1Edit ini mappings...** and pick one or more `.ini`
   files (Ctrl or Shift selects several).
2. Every line that names a map of the open profile becomes an override, as
   if you had chosen that LvlType and Def in **Properties > Map context**.
   They are saved per profile in `config.toml`; open maps are redrawn with
   their new tile set.
3. A notification says how many mappings were read, how many maps now use
   them and how many lines name maps the profile does not have. Lines that
   can't be read (a missing number, a path that is not a `.ds1`) are listed
   in the Log with their file and line number.

When a file lists a map twice, the later line wins. **Clear override** in
Properties undoes an imported mapping for one map.

---

## Viewing a map

<!-- screenshot: the Map panel at 1:2: the town with its objects and NPCs, one NPC selected with its path -->

When a map opens, the view fits the whole map in the panel. Until its tiles
are loaded you see flat coloured diamonds; then the textured map replaces
them.

### Zoom and pan

| Action | How |
|---|---|
| Zoom in or out at the cursor | Mouse wheel. Steps are 1:1, 1:2, 1:3, 1:4, 1:6, 1:8, 1:12 and 1:16, the same as DS1Edit. |
| Pan | Middle-drag, or hold **Space** and drag with the left button. |
| Pan with the keyboard | Arrow keys. |
| Fit the map to the panel | **View > Fit map to window**. |
| Cell grid | **G**, or **View > Cell grid**. The grid hides when cells are too small to see. |

The zoom is shown in the panel's bottom-left corner and in the status bar.
The status bar also shows the cell under the cursor, and **Properties >
Hovered cell** lists what each layer holds there (main and sub index,
orientation, tag value).

### Layers and special tiles

The **Layers** panel lists the map's layers: Floor 1-2, Shadow, Wall 1-4 and
Tag, as present in the map.

- The checkbox shows or hides a layer.
- Click a layer's name to make it the **active layer**. Paint, erase and most
  selection tools work on the active layer. A layer with selected cells shows
  how many.
- **Roofs** shows or hides roof tiles (orientation 15). On by default.
- **Special tiles** shows or hides the orientation 10 and 11 marker tiles.
  Off by default.
- **Objects** shows or hides objects and NPCs (sprites, shadows and
  markers). The count is next to it.

The Tag layer is always drawn as flat coloured diamonds, and it starts
hidden. You can change which layer kinds start visible in `config.toml` (see
[Advanced](#advanced-settings-and-environment-variables)).

### Objects

Objects and NPCs are drawn with their real sprites, in the game's drawing
order: their shadows, objects that stand behind walls, then each object
between the walls of its tile, as DS1Edit draws them. Transparent, additive
(flames, glows) and other blended layers look as they do in DS1Edit.

- **Animation.** **View > Animate objects** plays every object and NPC at its
  own game speed (on by default; the choice is saved). Turn it off to see
  the first frame of each animation, which is also what saved screenshots
  and DS1Edit's headless renders show.
- **Markers.** With the Objects tool (**O**) every object also gets an orange
  diamond marker on its sub-tile, a handle you can click even when the
  sprite is small or hidden behind a wall. **View > Object markers** shows
  them all the time. Objects that draw nothing (no sprite in this profile,
  or an id the game tables don't know) always get a **purple** marker.
- The object tables load in the background when a profile opens. Until
  they are ready, objects show as markers only.

To look at one object closely, select it with the Objects tool and turn on
**View > Sprite preview**: it shows the object's name, type, id, token, mode
and class, lets you pick a direction, play or pause, and **Open in Sprites
mode** to inspect its COF. Editing objects is described in
[Objects and NPC paths](#objects-and-npc-paths).

### Missing tiles

A cell whose tile isn't found in the map's DT1 set gets a thin **magenta
diamond outline**. **Properties > Rendering** shows how many tiles were drawn
and how many are missing. Hover the count to see the most common missing
tiles. See [Troubleshooting](#troubleshooting) for what to do.

---

## Editing a map

### Tools

The tool strip sits above the map. Hover a button for its name and key.

| Key | Tool | What it does |
|---|---|---|
| **M** | Select box | Drag a rectangle of cells. The size shows next to it. |
| **L** | Lasso | Drag a free shape. Cells whose centre is inside are selected. |
| **T** | Select by tile ID | Click a cell: selects every cell with the same main and sub index, whatever its orientation or flags. |
| **S** | Select identical | Click a cell: selects every cell with exactly the same value (indices, orientation and flag bits). |
| **P** | Paint | Paints the current tile on the active layer. |
| **E** | Erase | Empties cells on the active layer. |
| **I** | Eyedropper | Click a cell to make its tile the current tile. |
| **O** | Objects | Select, move and delete objects; add objects and NPCs; edit NPC paths. See [Objects and NPC paths](#objects-and-npc-paths). |

Tool keys work without modifiers. They are ignored while you type in a text
box or a dialog is open.

#### Selection modifiers

Hold a key when you start a selection with Select box, Lasso, Select by tile
ID or Select identical:

| Key held | Effect |
|---|---|
| none | Replace the selection |
| **Shift** | Add to it |
| **Ctrl** | Subtract from it |
| **Shift+Ctrl** | Intersect with it |
| **Alt** | Intersect with it (same as Shift+Ctrl) |

These are the same keys as in the Sprites editor. Shift or Ctrl alone wins
over Alt. Selected cells are tinted by layer: green for floors, blue for
walls, purple for the shadow, pink for tag.

#### Selection scope

When a selection tool is active, the **On** box next to the tools sets which
layers it looks at:

- **Active layer**: only the active layer (the default).
- **All visible layers**: every layer whose checkbox is on in the Layers
  panel.

Which one the editor starts with is set in **File > Settings...**.

Select by tile ID and Select identical read the clicked cell from the active
layer if it has a tile there. Otherwise they use the topmost layer in scope
that has one. Clicking an empty cell clears the selection (unless you hold a
modifier).

**Edit > Select all on the active layer** (**Ctrl+A**) selects every non-empty
cell of the active layer. **Ctrl+D** deselects.

#### Paint, erase and the eyedropper

1. Pick a tile with the Eyedropper (**I**). It takes the tile on the active
   layer under the cursor, or else the top layer that has one, and makes that
   layer active. For walls the orientation comes with the tile.
2. Switch to Paint (**P**) and drag over the map. The **Value** thumbnail in
   the tool strip shows what you paint.
3. Set the **Brush** size (1 to 16 cells, square) in the tool strip.

Each stroke is one undo step. Erase (**E**) uses the same brush and empties
cells on the active layer.

Every tile you pick is added to **Recent tiles** in the Tiles panel (the last
16). Click one there to make it current again; this also switches to Paint
and to the layer it came from.

### Copy, cut and paste

- **Ctrl+C** copies the selected cells, plus any objects inside the
  selection's bounding box.
- **Ctrl+X** cuts: copies, then empties the selected cells.
- **Ctrl+V** starts a floating **paste preview**.

Copies also go to the Windows clipboard, so you can paste between two running
copies of the editor and between maps of different profiles.

<!-- screenshot: a paste preview with green and red cells and the Paste preview panel -->

While the paste preview is active:

- the clip follows the mouse over the map;
- cells that will be written are **green**, and cells that would replace a
  different existing tile are **red** (conflicts);
- the **Paste preview** panel in the map's top-right corner counts:
  - **Writes**: cells that will change;
  - **Conflicts**: occupied cells with different content;
  - **Outside the map**: clip cells that fall off the edge and are dropped;
  - **Layers to add**: layers the map lacks and will get;
  - **Needs DS1 version**: whether the map must be upgraded first;
  - **Warning**: for example, the clip came from a different act or LvlType,
    so its tiles may not exist in this map's tile set.

Choose what happens on occupied cells under **When the target is occupied**:

| Mode | Effect |
|---|---|
| Overwrite | Every clip cell is written. |
| Skip occupied cells | Only empty target cells are written. |
| Only into empty layers | Only layers that are completely empty, or don't exist yet, are written. |

The mode you pick is remembered for the next paste. To finish:

- click the map, press **Enter**, or click **Paste here** to commit;
- press **Esc** or click **Cancel** to drop it.

### Moving a selection to another layer

Use this to move tiles between layers, for example Wall 1 to Wall 3:

1. Select the cells.
2. Choose **Edit > Move selection to layer...**.
3. Pick **From** (a layer that has selected cells) and **To**. Target layers
   the map doesn't have yet are marked **(new)** and are added
   automatically.
4. Choose what to do where the target is occupied: **Skip those cells** or
   **Overwrite them**.
5. Click **Move**.

A notification reports how many cells moved and how many were skipped. The
selection follows the moved cells to their new layer. Moving between a wall
layer and a floor or shadow layer drops the wall orientation.

### DS1 versions and "Upgrade to vN"

Older DS1 versions can't store everything. For example:

| Change | Lowest DS1 version |
|---|---|
| Objects | 2 |
| A second wall layer | 4 |
| Object flags | 6 |
| A tag layer | 10 |
| NPC paths | 14 |
| A second floor layer | 16 |

When an edit, paste, layer move or new layer needs a newer version than the
map has, the editor asks: **"This map needs a newer DS1 version"**. Click
**Upgrade this map to vN and continue** to upgrade the map and carry on with
the edit. The upgrade can be undone with **Ctrl+Z**. Sometimes the editor asks
for a version above the obvious one. For example, a version 3 map with tag
data must go to version 10, because versions 4 to 9 can't store a tag layer.

You can also upgrade any time from the Layers panel: open **Layer actions**
(or right-click a layer) and choose **Upgrade DS1 version (now vX) > Upgrade
to vN**. The highest version is 18.

The editor saves a map in its own version. It doesn't force version 18 the
way DS1Edit does.

### Cell flags

With cells selected, **Edit > Selection flags** sets or clears the **hidden**
and **unwalkable** bits of every selected cell.

### Resizing the map

**Edit > Resize map...** sets a new width and height (1 to 1024 cells). The
top-left corner stays in place. Shrinking cuts off cells beyond the new edge;
**Ctrl+Z** brings them back. Objects outside the new size are kept as they
are.

### Layer actions

Open **Layer actions** at the bottom of the Layers panel, or right-click a
layer name:

- **Add floor layer** (at most 2), **Add wall layer** (at most 4), **Add tag
  layer**;
- **Remove *layer*** (right-click menu only). The shadow layer can't be
  removed. Undo restores a removed layer with its content.
- **Upgrade DS1 version** (see above).

### Objects and NPC paths

<!-- screenshot: the Objects panel with the Add object picker and its preview, and an NPC path on the map -->

The Objects tool (**O**) has three modes, picked in the tool strip or at the
top of the **Objects** panel:

| Mode | A click on the map |
|---|---|
| **Select** | Selects the object under the cursor: its marker, or else the topmost sprite there. **Shift**-click adds or removes one. Drag a selected object to move all selected objects (they snap to sub-tiles, 5 per cell edge). Click empty ground to deselect. |
| **Add** | Places the object chosen in the picker on the sub-tile under the cursor (a green diamond and the sub-tile coordinates follow the cursor). The new object is selected. |
| **Path** | Edits the NPC path of the selected object (see below). With nothing selected, a click selects an object. |

**Delete** removes the selected objects (in Path mode: the selected path
point). Objects must be visible (the Objects checkbox in Layers) to be
picked.

#### Adding objects and NPCs

The **Add object** part of the Objects panel lists everything the game can
place in the map's act (type 2 objects such as torches, chests and
waypoints, and type 1 monsters and NPCs):

- **Search** filters the list. Every word must match the name, the sprite
  token, the class (`npc`, `object`), `type2` or `id37`, or `2:37`. A plain
  number also finds that id exactly, listed first.
- Click an entry to choose it: the Objects tool switches to **Add** and an
  animated preview of the object appears below the list.
- Click on the map to place it. Every click places another one.

Placing an object on a map older than DS1 version 2 offers the usual
[upgrade](#ds1-versions-and-upgrade-to-vn). Placing it on a sub-tile where
an object with an NPC path already stands is refused (the game could not
tell whose path it is), with a notification naming the sub-tile and the
objects.

#### Object properties

With one object selected, the Objects panel shows:

- the name from the game tables, the sprite token, mode, weapon class, order
  flag and where the data came from;
- whether it draws (and why not, when it doesn't);
- **Type**, **Id**, **Flags** and the **Sub-tile** position. Change them and
  click **Apply** (one undo step) or **Revert**. Flags need DS1 version 6;
  moving an object carries its NPC path along.

#### NPC paths

An NPC's walking route is a list of points with an action each. With one
object selected, its path is drawn on the map: a line from the object
through numbered blue nodes; an action other than 1 is shown as `a2`, `a3`
next to its node. The Objects panel lists the points with their position and
action.

In **Path** mode:

- click on empty ground to add a point there: after the selected node when
  one is selected, else at the end of the path. The new point is selected,
  so you can click on to extend the route;
- drag a node to move it;
- **Delete** removes the selected node;
- **New points** in the Objects panel sets the action new points get; the
  action combo of each point in the list changes that point.

The actions are numbered 1 to 5. Action 1 is the default; what the game does
with the others is not documented. Paths need DS1 version 14 and actions
other than 1 need version 15: on older maps the editor offers the upgrade.
Every change is one undo step. A path belongs to the object standing on its
first sub-tile, so an object that shares its sub-tile with another can't get
a path.

### Delete, undo and redo

- **Delete** removes the selected objects if any are selected (in the
  Objects tool's Path mode: the selected path point). Otherwise it empties
  the selected cells.
- **Ctrl+Z** undoes, **Ctrl+Y** or **Ctrl+Shift+Z** redoes. The Edit menu
  shows what will be undone or redone.

Undo is unlimited and separate for each open map. A tab shows **●** before
its name while it has unsaved changes. Undoing back to the saved state clears
the mark.

### Closing maps and quitting

Close a tab with its **x**, by middle-clicking it, or with **File > Close
map**. If the map has unsaved changes the editor asks: **Save**, **Discard**
or **Cancel**. Quitting asks the same for every unsaved map, and asks about
unsaved sprite edits first.

---

## Saving and packing

### Where files go

**Ctrl+S** (or **File > Save**) writes the active map to:

```
capybara\saves\<profile>\<game path>
```

For example, `data\global\tiles\act1\town\towne1.ds1` in the `vanilla`
profile is saved as:

```
capybara\saves\vanilla\data\global\tiles\act1\town\towne1.ds1
```

Folder and file names are lowercase. Each save:

1. writes the file safely (to a temporary file, then renamed into place), so
   a crash never leaves half a file;
2. adds the game path to `capybara\saves\<profile>\CHANGED.txt` if it isn't
   listed yet;
3. records a snapshot in `capybara\history\<profile>\`;
4. marks the map as coming from `saves` in the Maps list.

If the map holds something its DS1 version can't store, the save stops and
asks **"This map cannot be saved in its DS1 version"**. Click **Upgrade to vN
and save** to upgrade and save in one step.

### CHANGED.txt

`CHANGED.txt` lists, one per line, the game paths you have saved since you
last marked them as packed. **File > CHANGED.txt: N file(s)** shows how many.
Sprites saved with "Save to game path" are listed here too.

### Packing by hand

The editor never writes MPQ archives. Packing is up to you:

1. Choose **File > Open saves folder**. Explorer opens
   `capybara\saves\<profile>\`.
2. Open your mod's MPQ in an MPQ editor.
3. Drag the `data` folder from Explorer into the MPQ editor. The paths inside
   already match the game's paths, so each file lands where the game looks
   for it.
4. Check `CHANGED.txt` to see exactly which files are new or changed.

### Mark as packed

After packing, choose **File > Mark as packed (N)** and confirm. This empties
`CHANGED.txt`. Your saved files stay where they are, and they still override
the archives in the editor.

### Snapshots

Every save also stores a compressed copy of the file in
`capybara\history\<profile>\`. If you save the same content twice in a row,
the second save doesn't add a snapshot. The **History** panel shows them; see
the next section.

---

## History: autosave, checkpoints, diff and restore

<!-- screenshot: the History panel with a checkpoint selected and the diff overlay on the map -->

The editor keeps the versions of each map in `capybara\history\<profile>\`.
Identical content is stored once. There are four kinds of snapshot:

| Kind | Made by | Pruned by retention |
|---|---|---|
| Save | every save (**Ctrl+S**) | yes |
| Autosave | the autosave timer, or **Autosave now** | yes |
| Checkpoint | **Checkpoint...**, or **Make checkpoint...** on any snapshot | **never** |
| Before restore | a restore, just before it replaces the map | yes |

Only saves write `capybara\saves\` and `CHANGED.txt`. Autosaves, checkpoints
and restores never touch them, so nothing new needs packing until you save.

### Autosave

Every few minutes (5 by default; set it in **File > Settings...**, 0 turns it
off), each open map with **unsaved changes** is recorded as an Autosave
snapshot. Maps without unsaved changes are skipped, and so is a map that has
not changed since its last autosave. The writing happens in the background,
so the editor doesn't pause. The status bar and the History panel then show
`autosaved 14:32`. **Autosave now** in the History panel does it at once.

### The History panel

The panel shows the active map's timeline, **newest first**: the kind, the
name or label, the time (hover for the full date) and the size.

- **Checkpoint...** records the map as it is now under a name, without saving
  it. Use it before a risky change.
- **Milestone...**: see [Milestones](#milestones).
- Click a row to select it and compare it with the map (below).
  Right-click a row, or use the buttons under the list, for:
  - **Rename / label...**: a note shown in the timeline (empty removes it);
  - **Make checkpoint...** (or **Rename checkpoint...**): turns the snapshot
    into a named checkpoint, which retention never removes;
  - **Restore...** (below);
  - **Delete...**: removes it from the timeline. Its stored copy is freed by
    the next prune.

### Comparing with a snapshot

Selecting a snapshot compares it with the map as it is now (including unsaved
changes):

- **Show changes on the map** paints every differing cell: **green** where a
  tile was added since, **red** where one was removed, **yellow** where it
  changed (another tile, orientation or flags). Objects added or removed get
  a green or red ring.
- The summary counts the added, removed and changed cells of each layer. The
  checkbox in front of a layer shows or hides that layer's cells on the map,
  and **Jump** centres the map on the layer's next change (click again for
  the one after).
- Size, DS1 version, layers added or removed and other data are listed below
  the counts.
- **Show snapshot (read-only)** draws the snapshot in place of the map so you
  can flip between then and now. While it is on, the map can't be edited:
  a blue line at the top of the map says so, the mouse tools do nothing, and
  editing keys and menu items (Ctrl+Z, Ctrl+Y, Ctrl+X, Ctrl+V, Delete) are
  refused with the notification "Viewing a snapshot: turn it off to edit".
  Untick it to go back.

### Restore

**Restore...** asks first. Then:

1. the map as it is now is recorded as a **Before restore** snapshot;
2. the map is replaced by the snapshot, as **one undo step**: **Ctrl+Z**
   brings your version back.

The map is then marked unsaved. Restoring doesn't save; press **Ctrl+S** when
you are happy with it.

### Retention

The newest saves, autosaves and before-restore snapshots of each map are kept
(100 by default). You can also drop those older than a number of days.
Checkpoints are never removed. Set both in **File > Settings...**; the policy
is applied whenever a profile opens, and **Prune now** applies it at once and
reports how much was freed.

### Milestones

A milestone is a git commit of the whole `capybara\saves\` folder (every
profile), made with a built-in git library; you don't need git installed.
**Milestone...** asks for a message and commits every saved file, including
deletions since the last milestone. Unsaved changes are not included, so save
first. If nothing changed since the last milestone, the editor says "nothing
changed" and makes no commit. The **Milestones** list at the bottom of the
History panel shows the commits, newest first.

The repository is `capybara\saves\.git`. Any git tool can read it, for
example to see what changed between two milestones.

---

## Settings

<!-- screenshot: the Settings window -->

**File > Settings...** holds:

| Section | Setting |
|---|---|
| Map view | **Gamma** of the map view and the object preview (0.5 to 3.0, default 1.30). The map is redrawn live as you drag. **Reset** goes back to 1.30. |
| Layers visible when a map opens | Floors, Walls, Shadow, Tag, Objects. **Apply to the open map** applies them to the active map now. |
| Start-up | **Show loading screen**: the door animation while the editor loads (default on). |
| Selection | Whether selection tools start on the **Active layer** or **All visible layers**. Changing it also changes the current choice. |
| Autosave | The interval in minutes; 0 is off. |
| History retention | How many saves/autosaves to keep per map, and an optional age limit in days (0 = none). **Prune now**. |
| Game profiles | The profiles, **Add game install...** and **Open profiles folder**. |

Settings are saved in `config.toml` straight away.

---

## Sprites mode

Click the **Sprites** tab at the top of the window. Sprites mode has these
panels:

| Panel | What it does |
|---|---|
| Sprites | Every `.dc6`, `.dcc` and `.cof` in the profile, with a filter and **Open file...**. |
| Viewer | Plays a sprite or a COF composite. |
| Editor | Palette-locked pixel editor. |
| Converter | Save to game path, image import, exports, batch conversion. |
| Log | The same log as the Map Editor. |

### The sprite list

Paths are shown without `data\global\`. Use **Filter** (for example
`monsters\zm`) and the **DC6**, **DCC** and **COF** checkboxes to narrow the
list, then click a row to open it in the Viewer.

**Open file...** opens a DC6, DCC or COF from anywhere on disk. Such files
are opened read-only.

### The Viewer

<!-- screenshot: the Viewer showing a monster COF with the layer list on the left -->

- **Palette** picks the colours: Act 1 to Act 5, any other `pal.dat` in the
  profile (units, menus, and so on), or a built-in grey ramp. The Viewer and
  the Editor share this choice.
- **gamma** (0.5 to 3.0) changes only what you see. It never changes the
  pixel data.
- **Zoom** 1x to 16x, or the mouse wheel over the picture. Drag with the left
  or middle button to pan; double-click to re-centre.
- **Checker** shows a checkerboard behind the sprite. Turn it off to pick a
  solid background colour.
- **Box** draws each frame's box (yellow) and the box around all frames
  (blue). **Anchor** marks the anchor point with a red cross.
- **Direction**, **Play/Pause** (**Space**), the **Frame** slider, and **,**
  and **.** to step frames.
- **fps**: the playback speed. For game sprites it comes from AnimData (hover
  to see the source), and you can change it.

The bottom-left corner shows the frame's size, offset and the box size.

#### COF files

Opening a `.cof` builds the composite from its component sprites. A panel on
the left lists:

- **Mode / class**: switch to another mode and weapon class of the same
  token;
- one row per layer (component), with a checkbox to show or hide it, its
  weapon class, an armor variant box when there is a choice, and its blend
  mode for transparent layers. Problems such as a missing sprite show in
  orange.
- **Edit** on a layer opens that layer's sprite in the Editor.

Layers are drawn in the COF's own order for each frame. COFs can be viewed
but not converted or saved.

### The Converter

**Work on** chooses which sprite the Converter acts on: the **Viewer sprite**
or the **Editor sprite** (only when the Editor has one). Below it you see
that sprite's direction and frame counts and its format.

The **palette** shown here is the one imported colours are matched against
and exports use.

#### "Save to game path" and "Export to…"

These two write to different places:

| | Save to game path | Export to… and batch |
|---|---|---|
| Where | `capybara\saves\<profile>\<game path>` | A folder you pick |
| CHANGED.txt | Yes | No |
| Snapshot | Yes | No |
| Needs a profile | Yes | No |

**Save to game path...** asks for a game path. It must start with `data\` and
end in `.dc6` or `.dcc`; the extension picks the format. The box is filled
with the sprite's own game path when it has one, otherwise with
`data\global\<name>.<ext>`. The saved file is decoded once more to make sure
the game can read it.

#### Importing images

1. Click **Pick images...** (PNG, BMP, GIF or PCX). Several files become
   frames in name order. Or click **Import folder...** for a folder you
   exported earlier with `sprite.json`.
2. Choose the **Layout**:
   - **One image per frame**: set **Directions** and **Frames** (0 means
     "share the images out evenly between the directions"). The image count
     must equal directions × frames.
   - **Sheet (rows x columns)**: one image cut into rows (directions) and
     columns (frames).
3. Set colour matching:
   - **Dither**: None or Floyd-Steinberg;
   - **Transparent colour**: one colour to treat as transparent (on, black,
     by default);
   - **Alpha below**: pixels less opaque than this become transparent
     (default 128);
   - **Protect transparent**: an opaque pixel never becomes palette index 0,
     which is the transparent colour (on by default).
4. Optionally tick **Split frames larger than 256x256** (cut into 256×256
   pieces).
5. Choose **Encode as** DCC or DC6, and tick **Open in the editor too** if
   you want to edit the result.
6. Click **Import**. The result opens in the Viewer.

Offsets: when a `sprite.json` sits next to the images (or a `<name>.json`
next to a sheet), every frame gets back its exact offset, and the
transparent-colour setting is ignored because the exported PNGs already carry
transparency. Without one, each frame is anchored at its bottom centre.

#### Exports

| Button | Writes |
|---|---|
| Frames + sprite.json... | One PNG per frame (`d00_f000.png`, ...) and `sprite.json` with the exact offsets, for a lossless re-import. |
| Sprite sheet... | One PNG, rows are directions and columns are frames, plus a `.json` with the offsets. |
| Animated GIF... | The current direction at the Viewer's speed. |
| Indexed PNG... | The current frame as a palette PNG, for round trips through GIMP or Photoshop. |
| Sprite file (DC6/DCC)... | The sprite encoded in the format of the file name you choose. |
| Palette .gpl / .act / .pal | The current palette for GIMP, Photoshop or JASC tools. |

#### Batch conversion

1. Pick an **Input folder...** and an **Output folder...**.
2. Choose the direction:
   - **Sprites to PNG frames**: each sprite becomes a folder of frames plus
     `sprite.json`;
   - **Images to sprites**: each folder with a `sprite.json`, or each lone
     image, becomes one sprite. Set **Encode as**, the colour matching and
     splitting as for an import.
3. Tick **Include sub-folders** and **Overwrite existing outputs** as needed.
4. Click **Start batch**. A progress bar and **Cancel** appear.

The output folder mirrors the input folder's tree, and nothing is ever
written inside the input folder. The report goes to the Log.

### The Editor

<!-- screenshot: the sprite Editor with tools on the left, palette on the right and the frame strip below -->

Open a sprite in the Editor in one of three ways:

- **Open in editor** in the Viewer;
- **Edit** on a COF layer;
- an import with **Open in the editor too** ticked.

The Editor holds one sprite at a time. If it has unsaved changes, opening
another sprite asks first.

The top bar shows:

- the sprite's name (**●** while unsaved), format and size;
- **Undo**/**Redo**;
- **Save to game path...** and an **Export** menu (the same exports as the
  Converter);
- **Format** (DCC or DC6), palette and gamma;
- **zoom** (1x to 64x), **Grid** (from 4x), **Checker**, **Anchor**, and
  **Onion skin** with its **fade**.

#### The palette lock

DC6 and DCC sprites don't store colours. They store palette **indices** from
0 to 255, and the game looks the colours up in a palette. Index 0 means
transparent.

The Editor keeps that true at all times:

- you draw only with palette colours, picked from the palette grid;
- gradients, shading and pasted images are mapped to the nearest palette
  colours;
- an opaque pixel never becomes index 0, so nothing you draw turns invisible
  in game;
- to make pixels transparent, use the Eraser or **Delete**.

Switching the palette box (for example to see the sprite under the Act 3
palette) only changes how the indices look. It doesn't change any pixel.

#### Colours

The palette panel on the right has:

- **Primary** and **Secondary** swatches, and **Swap (X)**;
- **Transparent (eraser)**, which switches to the Eraser;
- the 16×16 palette grid: left-click sets the primary colour, right-click the
  secondary. Index 0 is crossed out because it is not a colour.
- **Ramps**: the palette's colour families, each sorted dark to light. Click
  a swatch to pick it. The ramp of the primary colour is outlined. Shading,
  ramp gradients and auto outlines step along these ramps.

On the canvas, the left mouse button uses the primary colour and the right
button the secondary.

#### Tools

| Key | Tool | Notes |
|---|---|---|
| **B** | Pencil | Brush size and Square or Round shape. |
| **E** | Eraser | Paints transparency. |
| **L** | Line | Drag from start to end. |
| **R** | Rectangle | Outline or Filled. |
| **C** | Ellipse | Outline or Filled. |
| **F** | Fill | **Contiguous** fills the touching area of the same index; **Global (same index)** fills every pixel of that index in the frame. |
| **K** | Replace colour | Click a colour to replace it with the primary (right-click: secondary): every pixel of that index changes. **In every frame** does it across the whole sprite. |
| **I** | Picker | Left-click picks the primary colour, right-click the secondary. |
| **G** | Gradient | See below. |
| **H** | Shade | Left button lightens, right button darkens, along each pixel's own ramp. **Steps** 1 to 8. |
| **M** | Select rectangle | |
| **U** | Select ellipse | |
| **Q** | Lasso | |
| **W** | Magic wand | Contiguous or Global, like Fill. |
| **V** | Move selection | Press inside the selection and drag. |
| **O** | Move offset | Drag to move the frame against the anchor. |

Brush size is 1 to 32 pixels; **[** and **]** change it.

When a selection exists, painting tools only change selected pixels. For the
selection tools, hold **Shift** to add, **Ctrl** to subtract, and
**Shift+Ctrl** to intersect (the same keys as in the Map Editor). The **Selection** buttons are **All**, **None**, **Invert**,
**Grow**, **Shrink**, **Flip H**, **Flip V**, **Rot CW** and **Rot CCW**.

#### Gradient

Drag from the start point to the end point. Options:

- **Linear** or **Radial**;
- **Two colours** (primary to secondary) or **Ramp** (along the primary
  colour's ramp, dark to light);
- **Dither**: None, Bayer or F-S (Floyd-Steinberg);
- **Lock alpha**: only repaint pixels that are already opaque.

The gradient is computed in full colour, then mapped to the palette. With
Ramp it only uses that ramp's colours.

#### Shading

The Shade tool (**H**) moves each pixel up or down its own colour ramp, so a
red pixel gets a lighter or darker red. Transparent pixels are left alone.

#### Outline and clean-up

Under **Effects**:

- **Auto darker** uses a darker shade of each edge pixel's own ramp (set the
  number of steps, negative is darker). Untick it to outline with the primary
  colour.
- **Outside** or **Inside** the shape; **Thickness** 1 to 8;
  **Diagonals**.
- **Outline** applies it. With a selection, it outlines the selection
  instead of the opaque pixels (on the current frame).
- **Clean strays** removes isolated single pixels and fills single-pixel
  holes. With a selection, only inside it (on the current frame).
- **All frames of this direction** applies Outline, Clean strays and offset
  drags to every frame of the current direction.

**Canvas** resizes the frame. The bottom-centre stays in place, so the anchor
doesn't move.

#### Frames and onion skin

The strip at the bottom shows the current direction's frames as thumbnails;
click one to go to it.

- **Direction** chooses the direction; **<** and **>** (or **,** and **.**)
  step frames.
- **Insert** adds a blank frame after the current one, and **Duplicate** a
  copy of it. **Delete** removes the current frame (the last remaining frame
  can't be deleted), and **Move left** and **Move right** reorder it.
- All four act on the same frame number in **every direction**, so all
  directions keep the same frame count.
- **Mirror into** a direction, then **Mirror direction** writes a
  horizontally mirrored copy of the current direction there.
- **Onion skin** (top bar) shows the previous and next frames faded behind
  the current one. **fade** sets how strongly.

#### Copy and paste

- **Ctrl+C** and **Ctrl+X** copy and cut the selection. The copy also goes to
  the Windows clipboard as an image.
- **Ctrl+V** pastes the editor's own clip. If there is none, it pastes an
  image from the Windows clipboard.
- **Ctrl+Shift+V** always pastes the Windows clipboard image, mapped to the
  palette.

Transparent pixels in a pasted clip don't overwrite what's below.

#### The DCC 4×4-cell limit

DCC compresses each direction as a grid of small cells, 4×4 pixels each. In
each frame, one cell can hold **at most 4 different values**, and transparent
counts as one of them. A cell that mixes, say, five shades of red can't be
stored exactly. DC6 has no such limit.

The **DCC cell check** at the bottom of the palette panel (open by default
for DCC sprites):

- says "Every 4x4 cell fits" in green, or how many cells hold more than 4
  values;
- lists each one as direction, frame, cell and value count. Click a line to
  jump there; the cell is highlighted.
- **Reduce all** merges the nearest colours in every crowded cell, as one
  undo step.

If you save or export as DCC while cells are over the limit, the encoder
reduces them on its own. It keeps transparency first, then the most frequent
colours, and moves the other pixels to the nearest kept colour. Before writing, the
editor asks: **Reduce and save**, **Open the cell check** to fix the cells
yourself first, or **Cancel**.

---

## The Tiles browser and Ctrl+K search

<!-- screenshot: the Tiles panel with a query, the scope buttons, Recent, Favorites and the thumbnail grid -->

The **Tiles** panel and the **Ctrl+K** search find any tile (and DS1, DT1,
DC6, DCC or COF file) of the profile by name, look or use, and turn a tile
into the paint brush.

### The Tiles panel

From top to bottom:

- **The query box.** Results update as you type. Free text matches the
  tile's name (an alias you gave it, or the generated name such as
  `Act1 Town Floor · Floor · grass · 05/02`), its path and its file; every
  word must match. Filter tokens narrow it down (table below). A query that
  can't be read is shown in red with a caret under the problem. **x**
  clears it.
- **Scope**: **This map's tile set** (only the DT1s the active map loads;
  needs a map whose LvlType is known), **Whole profile** (every tile), or
  **Sprites** (DC6, DCC and COF files). The number on the right is the
  thumbnail size: **Ctrl+mouse wheel** over the panel resizes the
  thumbnails (a list below 48 px).
- **Saved searches**: pinned searches as buttons (right-click to unpin);
  the **Saved searches** menu lists them all, pins, deletes, and **Save the
  current query...**.
- **Recent**: the tiles you painted with, newest first.
- **Favorites**: named groups of starred tiles. Drag tiles into a group or to
  reorder them, drag a group header to reorder the groups, right-click a
  header to rename or delete the group (the tiles stay in the index).
- **The results grid.** A tile from outside the active map's tile set has an
  orange **!** badge: it would be missing in this map.

On a thumbnail:

| Action | Effect |
|---|---|
| Click | Select it. |
| Double-click, or Enter | Use it: a tile becomes the brush (on its layer kind) and the Paint tool is picked; a map opens; a sprite opens in Sprites mode. |
| The star | Add to or remove from favorites (the first group, or a new "Favorites" group). |
| Right-click | The tile menu: **Use as brush / open**, **Set alias...**, **Add tag...** / **Remove tag**, **Add note...** / **Edit note...**, **Add to favorites** (a group, or **New group...**), **Remove from** a group, **Find similar**, **Show maps that use it**, **Copy key**. |

Aliases, tags, notes, favorites, saved searches and recents live in
`capybara\user.db`, keyed by the tile's game path and indices, so they
survive an index rebuild.

### Query tokens

| Token | Finds |
|---|---|
| `type:floor` (`wall`, `roof`, `shadow`, `special`, `tree`, `lowerwall`) | Tiles of that kind. |
| `type:tile`, `type:ds1`, `type:dt1`, `type:dc6`, `type:dcc`, `type:cof` | Assets of that kind. |
| `act:1` ... `act:5` | Tiles of that act. |
| `color:green` | Tiles whose dominant colour is that colour. |
| `tag:x`, `tag:"two words"` | Tiles you tagged. |
| `idx:5`, `idx:5/2`, `idx:0:5/2` | Main index, main/sub, or orientation:main/sub. |
| `file:floor.dt1` | Assets whose path contains the text. |
| `similar:<key>` | Tiles that look alike (nearest first); the tile menu's **Find similar** fills it in. |
| `used-in:towne1` | Tiles used by maps whose path contains the text. |
| `mirrored:yes` | Tiles with a mirrored twin. |
| `src:mod`, `src:base`, `src:saves`, `src:<archive>` | Where the asset comes from. |
| `-tag:foo` | A leading `-` negates any token. |

An unknown `word:` is plain text.

### Ctrl+K

**Ctrl+K** opens a search box in the middle of the window over every asset
of the profile, with live thumbnails. With an empty query it lists the
recently used tiles.

| Keys | Action |
|---|---|
| Up, Down, PgUp, PgDn | Move the selection (1 or 10 rows). |
| Enter | Use the asset: a tile becomes the brush, a map opens, a sprite opens in Sprites mode. |
| Shift+Enter | Show it in the Tiles browser. |
| Esc, or a click outside | Close. |

### The asset index

The browser and Ctrl+K search the asset index:

- When a profile opens, the editor checks `capybara\cache\index.db` in the
  background. If the index is missing or out of date, it rebuilds it.
- The status bar shows the progress, for example "Index: Indexing tiles 120/655
  ...", with a **Cancel** button.
- The index is rebuilt when an archive in the profile changes size or
  modification date. Otherwise the next start just reports "Index up to
  date".
- After you save, the index updates its view of your saved files in the
  background.
- Until the index is ready the Tiles panel and Ctrl+K say so; search starts
  when it is done.

From the command line, `index status` and `index build` do the same (see
[cli.md](cli.md)).

---

## Troubleshooting

### "Capybara's D2 Editor cannot use its data folder"

The editor couldn't create or write to `capybara\` next to the exe. The
message names the folder and the Windows error. Nothing was written anywhere
else. Fix it in one of two ways:

- move the exe to a folder you can write to (not `C:\Program Files\`), or
- set the environment variable `CAPY_HOME` to a writable folder, which is
  then used instead of `capybara\`.

### The editor stopped with an error message

If the editor stops unexpectedly it shows a message box titled
"Capybara's D2 Editor" and appends the details (time, version, the error,
where it happened and a backtrace) to `capybara\crash.log`. Please include
that file when you report the problem.

**"Unable to find a GPU"** means no graphics driver with Vulkan or DirectX 12
support was found: install the latest driver for your graphics card. Remote
desktop sessions and some virtual machines have no such driver.

### No profiles, or a profile won't open

- **"No game profiles"** in the Log: your installs aren't at the default
  locations. Add a profile file (see [Game profiles](#game-profiles)) and
  restart.
- **"Could not open profile"**: an archive in the profile doesn't exist, or
  an `mpq-glob` pattern matches nothing. Fix the path, or mark an optional
  archive with `optional = true`.
- A profile file with a syntax error, or with the same `id` as another file,
  is reported in the Log, and the other profiles still load.

### Missing tiles (magenta outlines)

Magenta diamond outlines mark cells whose tile isn't in the map's tile set.

1. Look at **Properties > Map context**. If **Context** is `DS1 header only`,
   the editor found no tile set at all; set one as described under
   [Overriding the LvlType and Def](#overriding-the-lvltype-and-def).
2. If an orange line says DT1 files were not found, the profile is missing
   archives or folders that hold them. Hover the line for the list and fix
   the profile.
3. If only a few tiles are missing, hover **Properties > Rendering > Missing
   tiles** to see which tile keys (orientation/main/sub) are missing. A mod
   may simply not ship those tiles.

### The map shows the wrong tiles

The catalog guessed the wrong LvlType, or the map has several LvlPrest
entries. In **Properties > Map context**, choose another **Interpretation**,
or pick a **LvlType** and **Def** and click **Apply**. Your choice is saved
for that map in this profile.

### Where is the Log?

In the Map Editor it is the bottom panel. In Sprites mode it is the **Log**
tab at the bottom. If you moved it and lost it, use **View > Reset panel
layout**.

### Other problems

- **The panel layout is broken**: **View > Reset panel layout**.
- **config.toml was unreadable**: the editor renames it to
  `config.toml.bad`, starts with default settings, and says so in the Log.
- **The asset index failed**: the status bar says so; hover it or read the
  Log. Deleting `capybara\cache\` forces a clean rebuild.
- **Ctrl+V says the clipboard holds no map cells**: copy cells first. Text
  or images on the Windows clipboard aren't map cells.

---

## Keyboard shortcuts

Shortcuts are ignored while you type in a text box or while a question
dialog is open.

### Map Editor

| Keys | Action |
|---|---|
| Ctrl+S | Save the active map |
| Ctrl+Z | Undo |
| Ctrl+Y, Ctrl+Shift+Z | Redo |
| Ctrl+C | Copy the selection |
| Ctrl+X | Cut the selection |
| Ctrl+V | Start the paste preview |
| Enter | Commit the paste preview |
| Ctrl+A | Select all on the active layer |
| Ctrl+D | Deselect (cells and objects) |
| Delete | Path mode: delete the selected path point; else delete the selected objects, else empty the selected cells |
| Esc | Cancel the paste preview, else the drag in progress, else close the Move/Resize window, else deselect |
| M | Select box |
| L | Lasso |
| T | Select by tile ID |
| S | Select identical |
| P | Paint |
| E | Erase |
| I | Eyedropper |
| O | Objects |
| G | Cell grid on/off |
| Ctrl+K | Search every asset (see [Ctrl+K](#ctrlk)) |
| Mouse wheel | Zoom at the cursor |
| Middle-drag, Space + drag | Pan |
| Arrow keys | Pan |
| Shift / Ctrl + selection | Add / subtract |
| Shift+Ctrl (or Alt) + selection | Intersect |
| Shift + click (Objects tool) | Add or remove an object from the selection |
| Ctrl + mouse wheel (Tiles panel) | Thumbnail size |

### Sprites mode

Which keys apply depends on the pointer. When the pointer is over the Viewer,
or the Editor has no sprite, **Space**, **,**, **.** and **Ctrl+S** act on
the Viewer. Otherwise all keys act on the Editor.

Viewer:

| Keys | Action |
|---|---|
| Space | Play / pause |
| , and . | Previous / next frame |
| Ctrl+S | Save the Viewer's sprite to a game path |
| Mouse wheel | Zoom |
| Left- or middle-drag | Pan |
| Double-click | Re-centre |

Editor:

| Keys | Action |
|---|---|
| Ctrl+Z | Undo |
| Ctrl+Y, Ctrl+Shift+Z | Redo |
| Ctrl+S | Save to game path |
| Ctrl+C, Ctrl+X | Copy, cut |
| Ctrl+V | Paste (the editor's clip, else the Windows clipboard image) |
| Ctrl+Shift+V | Paste the Windows clipboard image |
| Ctrl+A | Select all |
| Ctrl+D | Deselect |
| Ctrl+I | Invert the selection |
| Delete, Backspace | Make the selected pixels transparent |
| Esc | Cancel the drag in progress, else deselect |
| X | Swap primary and secondary colours |
| [ and ] | Smaller / larger brush |
| , and . | Previous / next frame |
| B | Pencil |
| E | Eraser |
| L | Line |
| R | Rectangle |
| C | Ellipse |
| F | Fill |
| K | Replace colour |
| I | Picker |
| G | Gradient |
| H | Shade |
| M | Select rectangle |
| U | Select ellipse |
| Q | Lasso |
| W | Magic wand |
| V | Move selection |
| O | Move offset |
| Ctrl + mouse wheel | Zoom at the cursor |
| Mouse wheel | Pan |
| Middle-drag, Space + drag | Pan |
| Right mouse button | Draw with the secondary colour |
| Shift / Ctrl / Shift+Ctrl + selection | Add / subtract / intersect |

---

## Advanced: settings and environment variables

### config.toml

The editor writes `capybara\config.toml` itself. If you edit it by hand, do
so while the editor is closed, because the editor rewrites the file when
settings change.

| Key | Meaning |
|---|---|
| `gamma` | Gamma of the map view and the object preview (default 1.30, DS1Edit's default). Also in **File > Settings...**. |
| `autosave_minutes` | Minutes between autosaves of maps with unsaved changes (default 5); `0` turns autosave off. |
| `keep_last` | History retention: newest saves/autosaves/before-restore snapshots kept per map (default 100, at least 1). |
| `max_age_days` | History retention: also drop those older than this many days; `0` (the default) means no age limit. |
| `select_scope` | `"active"` or `"visible"`: the layers selection tools start on. |
| `browser_thumb` | Thumbnail size of the Tiles browser in points. |
| `animate_objects` | **View > Animate objects** (default `true`). |
| `object_markers` | **View > Object markers** (default `false`). |
| `show_loading_screen` | **Settings > Show loading screen** (default `true`). |
| `[layer_visibility]` | `floor`, `wall`, `shadow`, `tag`, `objects`: which layer kinds start visible when a map opens. `tag` is `false` by default. |
| `last_profile` | The profile opened at start-up. |
| `mode` | `"map"` or `"sprites"`: the mode shown at start-up. |
| `[window]` | `width` and `height` of the window. |
| `[[recent_maps]]` | The File > Open recent list. |
| `[[overrides]]` | Your LvlType/Def choices, per profile and map, including the ones imported from DS1Edit `.ini` files. |

### Environment variables

| Variable | Effect |
|---|---|
| `CAPY_HOME` | Use this folder instead of `capybara\` next to the exe. The command line uses it too. |
| `CAPY_PROFILE` | Open this profile id at start-up instead of the last one. |
| `CAPY_OPEN_MAP` | Open this map (a game path) once the profile is ready. |
| `CAPY_NO_INDEX` | Set to `1` to skip building the asset index. |
| `CAPY_NO_CRASH_DIALOG` | Set to `1` to write `crash.log` without showing the crash message box. |

The editor also has hooks used by its automated tests, such as
`CAPY_SCRIPT`, which runs a file of editing commands and then exits (it has
history commands too: `autosave-now`, `checkpoint`, `history-select`,
`restore`, `milestone`, `prune`, `setting`, ..., and object commands:
`place-object`, `select-object`, `path-add`, `path-move`, `path-action`,
`path-delete`, `animate`, `object-frame-tick`, `import-ini`, ..., and
loading-screen commands: `splash`, `splash-save`, `splash-wait-closed`),
`CAPY_SCREENSHOT` and `CAPY_DEBUG_PANIC`. Runs driven by these hooks start
without the loading screen. They are meant for development and may change.
