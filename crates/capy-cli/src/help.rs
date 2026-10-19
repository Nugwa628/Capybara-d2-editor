//! `help [verb]`.

use crate::args::VERBS;
use crate::{CmdResult, Ctx, Fail};

const OVERVIEW: &str = "\
Capybara Map Editor command line

usage: <exe> <verb> [options]

verbs:
  render    render a DS1 map to a PNG without opening the editor
  index     show or build the asset index of a profile
  maps      list the maps of a profile
  profiles  list the profiles in capybara\\profiles
  help      this text; `help <verb>` for one verb (also --help, -h, /?)

Without a verb the editor starts. Game folders are never written to.
Exit codes: 0 ok, 1 runtime error (including a map whose tile set cannot be
found), 2 usage error (a bad or missing flag or value).";

const RENDER: &str = "\
render <map> --profile <id|path.toml> [--lvltype N [--def N]] [--zoom 1:N] [--gamma G]
       [--layers LIST] [--variant reference|documented] [--object-frame N]
       --out <file.png>

  <map>      a game path (data\\global\\tiles\\...\\x.ds1, resolved through the profile's
             layers including its saves folder) or a filesystem path to a .ds1
  --profile  a profile id (capybara\\profiles\\<id>.toml) or a path to a profile TOML
  --lvltype  LvlTypes id; with --def, the LvlPrest Def (DS1Edit's command-line form).
             Without it the editor's saved override for the map (capybara\\config.toml)
             is used, else the best LvlPrest/LvlType match. A .ds1 outside the profile
             needs one of the two.
  --zoom     1:N, N from 1 to 16 (default 1:1)
  --gamma    palette gamma (default 1.30, the reference's)
  --layers   comma list of floors, walls, shadow, roofs, special, objects, all
             (default: everything except special; without objects, terrain only)
  --variant  reference (DS1Edit's choice among tile variants, default) or documented
  --object-frame
             animation frame of every object sprite, 0 to 255, wrapped at each
             sprite's frame count (default 0)
  --out      PNG file to write

Prints the map info line, then the result line with the number of missing
tiles and of drawn and missing objects. Exit 0 even with missing tiles or
objects (a warning is printed); 1 when the tile set cannot be found, or the
render or the write fails.";

const INDEX: &str = "\
index status --profile <id|path.toml>
index build  --profile <id|path.toml>

  status  prints `missing`, `up to date` or `stale: <reason>`
  build   (re)builds the asset index in capybara\\cache, printing progress lines";

const MAPS: &str = "\
maps list --profile <id|path.toml> [--filter S]

  Prints one game path per line, a tab, and the MapCatalog source of the
  map's LvlType (Override, LvlPrest, PathGuess, TileCoverage, Ds1Header).
  Override is an LvlType saved in the editor (capybara\\config.toml).
  --filter keeps paths containing S (case-insensitive).";

const PROFILES: &str = "\
profiles list

  Prints each profile in capybara\\profiles: id, game version (1.13c, 1.14d
  or d2r), name.";

const HELP: &str = "\
help [verb]

  Also --help, -h or /? as the first argument.";

fn text_for(verb: &str) -> Option<&'static str> {
    match verb {
        "render" => Some(RENDER),
        "index" => Some(INDEX),
        "maps" => Some(MAPS),
        "profiles" => Some(PROFILES),
        "help" => Some(HELP),
        _ => None,
    }
}

/// Reminder printed after a usage error.
pub fn usage_line(verb: Option<&str>) -> String {
    match verb.and_then(text_for) {
        Some(text) => text.to_owned(),
        None => format!("verbs: {}; try `help`", VERBS.join(", ")),
    }
}

pub fn print(ctx: &mut Ctx<'_>, verb: Option<&str>) -> CmdResult {
    match verb {
        None => {
            ctx.say(OVERVIEW);
            Ok(())
        }
        Some(v) => match text_for(v) {
            Some(text) => {
                ctx.say(text);
                Ok(())
            }
            None => Err(Fail::Usage(format!(
                "no such verb {v:?}; verbs: {}",
                VERBS.join(", ")
            ))),
        },
    }
}
