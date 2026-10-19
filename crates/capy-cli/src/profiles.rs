//! Profile lookup: `--profile <id|path.toml>` and `profiles list`.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use d2_assets::{GameVersion, Overlay, Profile};

use crate::{CmdResult, Ctx, Fail};

fn looks_like_path(arg: &OsStr) -> bool {
    let text = arg.to_string_lossy();
    text.to_ascii_lowercase().ends_with(".toml") || text.contains(['\\', '/'])
}

/// Where `arg` points: a TOML path, or `profiles\<id>.toml`.
fn locate(ctx: &Ctx<'_>, arg: &OsStr) -> PathBuf {
    if looks_like_path(arg) {
        PathBuf::from(arg)
    } else {
        let mut file = arg.to_owned();
        file.push(".toml");
        ctx.profiles_dir().join(file)
    }
}

/// Reads the profile named by `arg` (an id or a TOML path).
pub fn load(ctx: &Ctx<'_>, arg: &OsString) -> Result<Profile, Fail> {
    let path = locate(ctx, arg);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        let hint = if looks_like_path(arg) {
            String::new()
        } else {
            format!(
                " (profiles are TOML files in {}; `profiles list` shows them)",
                ctx.profiles_dir().display()
            )
        };
        Fail::Runtime(format!("cannot read profile {}: {e}{hint}", path.display()))
    })?;
    Profile::from_toml(&text).map_err(|e| Fail::Runtime(format!("profile {}: {e}", path.display())))
}

/// Opens every layer of `profile` (read-only), with the saves root of this
/// `capybara\` folder.
pub fn open(ctx: &Ctx<'_>, profile: &Profile) -> Result<Overlay, Fail> {
    profile
        .open(&ctx.saves_dir())
        .map_err(|e| Fail::Runtime(format!("opening profile {:?}: {e}", profile.id)))
}

/// The profile's saves folder, `capybara\saves\<id>`.
pub fn saves_of(ctx: &Ctx<'_>, profile: &Profile) -> PathBuf {
    profile.saves_dir(&ctx.saves_dir())
}

/// The version as profile TOMLs spell it: `1.13c`, `1.14d` or `d2r`.
pub fn version_label(version: GameVersion) -> &'static str {
    match version {
        GameVersion::V113c => "1.13c",
        GameVersion::V114d => "1.14d",
        GameVersion::D2r => "d2r",
    }
}

pub fn list(ctx: &mut Ctx<'_>) -> CmdResult {
    let dir = ctx.profiles_dir();
    let profiles = Profile::load_dir(&dir)
        .map_err(|e| Fail::Runtime(format!("reading {}: {e}", dir.display())))?;
    if profiles.is_empty() {
        ctx.say(format!("no profiles in {}", dir.display()));
        return Ok(());
    }
    for p in &profiles {
        ctx.say(format!(
            "{}\t{}\t{}",
            p.id,
            version_label(p.game_version),
            p.name
        ));
    }
    Ok(())
}
