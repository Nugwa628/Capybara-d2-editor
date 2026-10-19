//! `render`: one DS1 to one PNG, headless.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use capy_render::{MissingObject, ObjectSprites, RenderOptions, TileSet, render_with_objects};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, GameVersion, MapCatalog, MapInfo, MapInfoSource, Overlay};
use d2_formats::ds1::Ds1;
use d2_formats::palette::{Palette, act_palette_path};

use crate::args::RenderArgs;
use crate::maps_cmd::{MAX_WARNINGS, warn_limited};
use crate::{CmdResult, Ctx, Fail, overrides, profiles};

/// Where the DS1 comes from.
enum Source {
    /// A game path that resolves in the profile's overlay.
    Game(GamePath),
    /// A file on disk, with the game path derived from its location (inside
    /// the profile's saves folder, or below a `data` folder) when possible.
    File { derived: Option<GamePath> },
}

/// The game path of a file on disk: relative to the profile's saves folder
/// when it is inside it, else the part from its first `data` folder on.
fn game_path_of_file(file: &Path, saves: &Path) -> Option<GamePath> {
    let file = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let saves = std::fs::canonicalize(saves).unwrap_or_else(|_| saves.to_path_buf());
    let parts = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect()
    };
    let tail: Vec<String> = if let Ok(rel) = file.strip_prefix(&saves) {
        parts(rel)
    } else {
        let all = parts(&file);
        let i = all.iter().position(|c| c.eq_ignore_ascii_case("data"))?;
        all[i..].to_vec()
    };
    if tail.is_empty() {
        return None;
    }
    Some(GamePath::new(&tail.join("\\")))
}

fn read_map(a: &RenderArgs, overlay: &Overlay, saves: &Path) -> Result<(Source, Vec<u8>), Fail> {
    let as_game = GamePath::new(&a.map.to_string_lossy());
    if overlay.resolve(&as_game).is_some() {
        let bytes = overlay
            .read_bytes(&as_game)
            .map_err(|e| Fail::Runtime(format!("reading {as_game}: {e}")))?
            .ok_or_else(|| Fail::Runtime(format!("{as_game}: not found")))?;
        return Ok((Source::Game(as_game), bytes));
    }
    let path = Path::new(&a.map);
    if path.is_file() {
        let bytes = std::fs::read(path)
            .map_err(|e| Fail::Runtime(format!("cannot read {}: {e}", path.display())))?;
        let derived = game_path_of_file(path, saves);
        return Ok((Source::File { derived }, bytes));
    }
    Err(Fail::Runtime(format!(
        "{}: not a file, and not a game path found in the profile",
        a.map.to_string_lossy()
    )))
}

fn resolve_info(
    a: &RenderArgs,
    overlay: &Overlay,
    profile: &d2_assets::Profile,
    source: &Source,
    ctx: &mut Ctx<'_>,
) -> Result<MapInfo, Fail> {
    let (lookup, from_file) = match source {
        Source::Game(g) => (g.clone(), false),
        Source::File { derived: Some(g) } => (g.clone(), true),
        Source::File { derived: None } => (
            GamePath::new(&Path::new(&a.map).file_name().map_or_else(
                || "map.ds1".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            )),
            true,
        ),
    };
    // Coverage evidence is only available for DS1s the overlay holds.
    let names: Vec<GamePath> = if overlay.resolve(&lookup).is_some() {
        vec![lookup.clone()]
    } else {
        Vec::new()
    };
    let (mut catalog, warnings) = MapCatalog::build(overlay, profile.game_version, &names)
        .map_err(|e| Fail::Runtime(format!("building the map catalog: {e}")))?;
    warn_limited(ctx, &warnings);
    if let Some(lvltype) = a.lvltype {
        return catalog
            .resolve_explicit(&lookup, lvltype, a.def)
            .map_err(|e| Fail::Runtime(format!("--lvltype {lvltype}: {e}")));
    }
    // The editor's saved choice for this map, as when it opens the profile.
    overrides::apply(ctx, &mut catalog, &profile.id);
    match catalog.best(&lookup) {
        Some(info) if !(from_file && info.source == MapInfoSource::Ds1Header) => {
            if info.source == MapInfoSource::Ds1Header {
                ctx.warn(format!(
                    "{lookup}: no LvlType found for this map; pass --lvltype (tiles will be missing)"
                ));
            }
            Ok(info)
        }
        // The arguments are well-formed; the map just has no known tile set.
        _ => Err(Fail::Runtime(format!(
            "{} is not part of the profile, so its tilesets are unknown: pass --lvltype N [--def N]",
            a.map.to_string_lossy()
        ))),
    }
}

fn load_palette(overlay: &Overlay, act: u8) -> Result<Palette, Fail> {
    let path = GamePath::new(act_palette_path(act));
    let bytes = overlay
        .read_bytes(&path)
        .map_err(|e| Fail::Runtime(format!("reading {path}: {e}")))?
        .ok_or_else(|| Fail::Runtime(format!("{path}: not found in the profile")))?;
    Palette::parse_dat(&bytes).map_err(|e| Fail::Runtime(format!("{path}: {e}")))
}

fn info_line(map: &str, info: &MapInfo) -> String {
    let opt = |v: Option<u32>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
    let coverage = info
        .coverage
        .map_or_else(|| "-".to_owned(), |c| format!("{c:.3}"));
    format!(
        "map {map}: source={:?} lvltype={} def={} act={} palette=act{} dt1s={} coverage={coverage}",
        info.source,
        opt(info.lvl_type),
        opt(info.lvlprest_def),
        u32::from(info.act) + 1,
        u32::from(info.palette_act) + 1,
        info.dt1_files.len(),
    )
}

/// Object counts of one render: `drawn` objects have a sprite, `missing`
/// ones lie inside the map but draw nothing, `outside` ones lie outside the
/// map's cells and are skipped (as in the reference).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectCounts {
    pub drawn: usize,
    pub missing: usize,
    pub outside: usize,
}

impl ObjectCounts {
    pub fn of(ds1: &Ds1, missing: usize) -> ObjectCounts {
        let max_x = i64::from(ds1.width) * 5;
        let max_y = i64::from(ds1.height) * 5;
        let inside = ds1
            .objects
            .iter()
            .filter(|o| {
                (0..max_x).contains(&i64::from(o.x)) && (0..max_y).contains(&i64::from(o.y))
            })
            .count();
        ObjectCounts {
            drawn: inside.saturating_sub(missing),
            missing,
            outside: ds1.objects.len() - inside,
        }
    }
}

/// The `objects: ...` part of the result line; `None` when the objects
/// layer is hidden.
pub fn objects_summary(counts: Option<ObjectCounts>) -> String {
    match counts {
        None => "objects: off".to_owned(),
        Some(c) => format!("objects: {} drawn, {} missing", c.drawn, c.missing),
    }
}

/// The object catalog of the profile, or `None` (with a warning) when it
/// cannot be built: objects are then reported missing, never fatal.
fn object_catalog(
    ctx: &mut Ctx<'_>,
    overlay: &Overlay,
    version: GameVersion,
) -> Option<ObjectCatalog> {
    match ObjectCatalog::build(overlay, version) {
        Ok((catalog, warnings)) => {
            warn_limited(ctx, &warnings);
            Some(catalog)
        }
        Err(e) => {
            ctx.warn(format!(
                "objects are not drawn: building the object catalog: {e}"
            ));
            None
        }
    }
}

fn warn_missing_objects(ctx: &mut Ctx<'_>, missing: &[MissingObject]) {
    if missing.is_empty() {
        return;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in missing {
        *counts
            .entry(format!("type {} id {} ({})", m.kind, m.id, m.reason))
            .or_default() += 1;
    }
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let top: Vec<String> = sorted
        .iter()
        .take(MAX_WARNINGS)
        .map(|(what, n)| format!("{what} x{n}"))
        .collect();
    let more = if sorted.len() > MAX_WARNINGS {
        format!(", and {} more kinds", sorted.len() - MAX_WARNINGS)
    } else {
        String::new()
    };
    let what = if missing.len() == 1 {
        "object draws"
    } else {
        "objects draw"
    };
    ctx.warn(format!(
        "{} {what} nothing: {}{more}",
        missing.len(),
        top.join(", ")
    ));
}

pub fn run(ctx: &mut Ctx<'_>, a: &RenderArgs) -> CmdResult {
    let profile = profiles::load(ctx, &a.profile)?;
    let overlay = profiles::open(ctx, &profile)?;
    let saves = profiles::saves_of(ctx, &profile);

    let (source, bytes) = read_map(a, &overlay, &saves)?;
    let ds1 = Ds1::parse(&bytes).map_err(|e| {
        Fail::Runtime(format!(
            "{}: not a readable DS1: {e}",
            a.map.to_string_lossy()
        ))
    })?;
    let info = resolve_info(a, &overlay, &profile, &source, ctx)?;
    let map_name = match &source {
        Source::Game(g) => g.to_string(),
        Source::File { .. } => a.map.to_string_lossy().into_owned(),
    };
    ctx.say(info_line(&map_name, &info));
    for missing in &info.missing_dt1 {
        ctx.warn(format!("{missing}: not found in the profile"));
    }

    let (tiles, warnings) = TileSet::load(&overlay, &info.dt1_files)
        .map_err(|e| Fail::Runtime(format!("loading tilesets: {e}")))?;
    let tiles = tiles.with_variant_rule(a.variant);
    warn_limited(ctx, &warnings);
    let palette = load_palette(&overlay, info.palette_act)?;

    let draw_objects = a.visibility.objects && !ds1.objects.is_empty();
    let catalog = if draw_objects {
        object_catalog(ctx, &overlay, profile.game_version)
    } else {
        None
    };
    let sprites = catalog
        .as_ref()
        .map(|catalog| ObjectSprites::new(&overlay, catalog));
    let opts = RenderOptions {
        zoom_div: a.zoom,
        gamma: a.gamma,
        visibility: a.visibility.clone(),
        object_frames: (a.object_frame != 0)
            .then(|| vec![Some(a.object_frame); ds1.objects.len()].into()),
        ..RenderOptions::default()
    };
    let rendered = render_with_objects(&ds1, &tiles, sprites.as_ref(), &palette, &opts)
        .map_err(|e| Fail::Runtime(format!("rendering: {e}")))?;
    warn_limited(ctx, &rendered.warnings);
    let object_counts = a.visibility.objects.then(|| match &sprites {
        Some(_) => ObjectCounts::of(&ds1, rendered.missing_objects.len()),
        // No catalog (or no objects): every object inside the map is missing.
        None => ObjectCounts::of(&ds1, ObjectCounts::of(&ds1, 0).drawn),
    });
    let png = rendered
        .image
        .to_png()
        .map_err(|e| Fail::Runtime(format!("encoding the PNG: {e}")))?;
    std::fs::write(&a.out, png)
        .map_err(|e| Fail::Runtime(format!("cannot write {}: {e}", a.out.display())))?;

    let missing = rendered.missing.len();
    ctx.say(format!(
        "rendered 1:{} gamma {:.2}: {}x{} px, origin {},{}, missing tiles: {missing}, {}, wrote {}",
        a.zoom,
        a.gamma,
        rendered.image.width,
        rendered.image.height,
        rendered.origin.0,
        rendered.origin.1,
        objects_summary(object_counts),
        a.out.display()
    ));
    if missing > 0 {
        let mut counts: BTreeMap<(char, u8, u8, u8), usize> = BTreeMap::new();
        for m in &rendered.missing {
            *counts
                .entry((m.layer.0, m.key.orientation, m.key.main, m.key.sub))
                .or_default() += 1;
        }
        let mut sorted: Vec<_> = counts.into_iter().collect();
        sorted.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let top: Vec<String> = sorted
            .iter()
            .take(MAX_WARNINGS)
            .map(|((l, o, m, s), n)| format!("{l}:o{o}/m{m}/s{s} x{n}"))
            .collect();
        ctx.warn(format!(
            "{missing} tiles have no matching DT1 tile (orientation/main/sub): {}",
            top.join(", ")
        ));
    }
    warn_missing_objects(ctx, &rendered.missing_objects);
    if let Some(c) = object_counts
        && c.outside > 0
    {
        let what = if c.outside == 1 {
            "object lies"
        } else {
            "objects lie"
        };
        ctx.warn(format!("{} {what} outside the map, not drawn", c.outside));
    }
    Ok(())
}
