# Command line

`CapybaraMapEditor.exe` is also a command-line tool. When the first argument
is one of the verbs below, the exe runs it and exits without opening a
window. With no arguments, or a first argument that isn't a verb, the editor
starts as usual.

```
CapybaraMapEditor.exe <verb> [options]
```

| Verb | What it does |
|---|---|
| [`render`](#render) | Render a DS1 map to a PNG |
| [`index`](#index) | Show or build a profile's asset index |
| [`maps`](#maps) | List a profile's maps |
| [`profiles`](#profiles) | List the profiles |
| [`help`](#help) | Show help, for all verbs or one |

All verbs only read game data. They write only the `--out` file you name and
files in `capybara\cache\`.

## General rules

- Verbs, subcommands and flag names are not case-sensitive.
- Every flag takes a value, as `--flag value` or `--flag=value`. A value
  can't start with `--`.
- Giving the same flag twice, or an unknown flag, is a usage error.
- There is no `--help` flag. Use `help` or `help <verb>`. (`--help` isn't a
  verb, so it starts the editor.)
- Output goes to stdout. Errors go to stderr as `error: ...`, and warnings
  as `warning: ...`.

### Where the CLI looks for profiles

The CLI uses the same `capybara\` folder as the editor: the one next to the
exe, or the folder in the `CAPY_HOME` environment variable when it is set.

`--profile` takes either:

- a profile **id**, read from `capybara\profiles\<id>.toml`; or
- a **path** to a profile TOML file. Anything ending in `.toml`, or
  containing `\` or `/`, is treated as a path.

The profile's `saves` layer points at `capybara\saves\<id>\`, so maps you
saved in the editor are used here too.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Runtime error: a file can't be read or written, a profile won't open, the render failed |
| 2 | Usage error: a missing or bad argument. The verb's usage is printed after the error. |

### Running from cmd or PowerShell

The release exe is a Windows GUI program, so it has no console window of its
own. When you run a verb from a console, it attaches to that console to print
its output. Because Windows doesn't wait for GUI programs, an interactive cmd
prompt can come back **before the output has finished printing**. The output
then appears after the prompt. The command still ran normally.

Redirection and exit codes work:

```
CapybaraMapEditor.exe maps list --profile vanilla > maps.txt
```

To make an interactive cmd prompt wait and then read the exit code:

```
start "" /wait CapybaraMapEditor.exe profiles list
echo %ERRORLEVEL%
```

Batch files wait for the program by themselves. In PowerShell, piping the
output (for example `| Out-Host`) makes it wait and sets `$LASTEXITCODE`.

Development builds (`cargo build` without `--release`) are console programs
and don't have this quirk.

---

## render

Render one map to a PNG, the same way the editor draws its terrain.

```
render <map> --profile <id|path.toml> [--lvltype N [--def N]] [--zoom 1:N]
       [--gamma G] [--layers LIST] [--variant reference|documented] --out <file.png>
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
| `--lvltype N` | best match | Use LvlTypes id N instead of the map catalog's choice. A `.ds1` that isn't part of the profile needs it. |
| `--def N` | none | LvlPrest Def to use with `--lvltype` (DS1Edit's command-line form). Needs `--lvltype`. |
| `--zoom 1:N` | `1:1` | Zoom divisor N from 1 to 16. `1:4` and `4` mean the same. |
| `--gamma G` | `1.30` | Palette gamma, above 0 and up to 10. 1.30 is DS1Edit's default. |
| `--layers LIST` | everything except `special` | Comma-separated: `floors`, `walls`, `shadow`, `roofs`, `special`, `objects`, or `all` (everything including `special`). |
| `--variant` | `reference` | How to choose among tile variants: `reference` matches DS1Edit; `documented` takes the first tile with the highest rarity. |

Without `--lvltype`, the tile set is chosen the same way the editor does (see
[How the tile set and palette are chosen](user-guide.md#how-the-tile-set-and-palette-are-chosen)).
Overrides you set in the editor are **not** used here; pass `--lvltype`
instead.

The render is terrain only. `objects` is accepted in `--layers`, but object
sprites aren't drawn yet (**planned**).

### Output

The first line describes the map context:

```
map <map>: source=<Source> lvltype=<N> def=<N> act=<N> palette=act<N> dt1s=<N> coverage=<C>
```

The last line reports the render:

```
rendered 1:<N> gamma <G>: <W>x<H> px, origin <X>,<Y>, missing tiles: <N>, wrote <file>
```

Missing tiles don't make the command fail. It still exits with 0, and prints
a warning listing the most common missing tiles as
`<layer>:o<orientation>/m<main>/s<sub> x<count>`. Missing DT1 files are
warned about too.

If the map isn't part of the profile and its tile set can't be found, the
command stops with exit code 2 and asks for `--lvltype`.

### Examples

Render a vanilla town map at 1:2:

```
CapybaraMapEditor.exe render data\global\tiles\act1\town\towne1.ds1 --profile vanilla --zoom 1:2 --out towne1.png
```

Render a map file from disk with an explicit LvlType and Def, floors and
walls only:

```
CapybaraMapEditor.exe render C:\work\mymap.ds1 --profile mxl --lvltype 2 --def 40 --layers floors,walls --out mymap.png
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
CapybaraMapEditor.exe index status --profile mxl
CapybaraMapEditor.exe index build --profile mxl
```

---

## maps

List a profile's maps, with how each one's tile set is found.

```
maps list --profile <id|path.toml> [--filter S]
```

Each line has a game path, a tab, and the map catalog source of that map's
LvlType: `LvlPrest`, `PathGuess`, `TileCoverage` or `Ds1Header` (see
[the user guide](user-guide.md#how-the-tile-set-and-palette-are-chosen) for
what each means). The overrides you set in the editor aren't read here, so
`Override` doesn't appear. `--filter` keeps paths that contain `S`, ignoring
case. If nothing matches, nothing is printed.

The list includes maps in your saves folder. The first run for a profile
gathers file names from the archives and caches them in
`capybara\cache\known-names-<id>.txt`, shared with the editor. Later runs
are faster.

### Example

```
CapybaraMapEditor.exe maps list --profile vanilla --filter town
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
vanilla	V114d	Diablo II 1.14d
```

The version is printed as `V113c`, `V114d` or `D2r`. With no profiles it
prints `no profiles in <folder>`. A profile file that can't be read is an
error (exit code 1).

---

## help

```
help [verb]
```

Without a verb, prints an overview of all verbs and the exit codes. With a
verb (`render`, `index`, `maps`, `profiles` or `help`), prints that verb's
usage. An unknown verb is a usage error.
