//! `maps list`.

use std::ffi::OsString;

use d2_assets::{GamePath, MapCatalog};

use crate::{CmdResult, Ctx, Fail, names, overrides, profiles};

/// How many catalog warnings are echoed before the rest is summarised.
pub const MAX_WARNINGS: usize = 5;

pub fn warn_limited(ctx: &mut Ctx<'_>, warnings: &[String]) {
    for w in warnings.iter().take(MAX_WARNINGS) {
        ctx.warn(w);
    }
    if warnings.len() > MAX_WARNINGS {
        ctx.warn(format!(
            "... and {} more warnings",
            warnings.len() - MAX_WARNINGS
        ));
    }
}

pub fn list(ctx: &mut Ctx<'_>, profile_arg: &OsString, filter: Option<&str>) -> CmdResult {
    let profile = profiles::load(ctx, profile_arg)?;
    let overlay = profiles::open(ctx, &profile)?;
    let known = names::known_names(ctx, &profile, &overlay)?;
    let needle = filter.map(str::to_lowercase);
    let mut maps: Vec<GamePath> = known
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ds1")))
        .filter(|p| {
            needle
                .as_ref()
                .is_none_or(|n| p.as_str().contains(n.as_str()))
        })
        .cloned()
        .collect();
    maps.sort();
    if maps.is_empty() {
        return Ok(());
    }
    let (mut catalog, warnings) = MapCatalog::build(&overlay, profile.game_version, &maps)
        .map_err(|e| Fail::Runtime(format!("building the map catalog: {e}")))?;
    warn_limited(ctx, &warnings);
    overrides::apply(ctx, &mut catalog, &profile.id);
    for map in &maps {
        let source = catalog
            .best(map)
            .map_or_else(|| "unknown".to_owned(), |i| format!("{:?}", i.source));
        ctx.say(format!("{map}\t{source}"));
    }
    Ok(())
}
