//! Command-line parsing. Pure: no I/O, no environment.

use std::ffi::OsString;
use std::path::PathBuf;

use capy_render::{VariantRule, Visibility};

/// A usage error: the arguments are wrong (exit code 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageError {
    pub message: String,
    /// Verb whose help text applies, if known.
    pub verb: Option<&'static str>,
}

fn usage(verb: &'static str, message: impl Into<String>) -> UsageError {
    UsageError {
        message: message.into(),
        verb: Some(verb),
    }
}

/// `render <map> ...`
#[derive(Clone, Debug, PartialEq)]
pub struct RenderArgs {
    /// Game path or filesystem path of the DS1.
    pub map: OsString,
    /// Profile id or path to a profile TOML.
    pub profile: OsString,
    pub lvltype: Option<u32>,
    pub def: Option<u32>,
    /// Zoom divisor N of `1:N`, 1..=16.
    pub zoom: u32,
    pub gamma: f32,
    pub visibility: Visibility,
    pub variant: VariantRule,
    /// Animation frame shown for every object sprite (taken modulo each
    /// COF's frame count).
    pub object_frame: u8,
    pub out: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Render(RenderArgs),
    IndexStatus {
        profile: OsString,
    },
    IndexBuild {
        profile: OsString,
    },
    MapsList {
        profile: OsString,
        filter: Option<String>,
    },
    ProfilesList,
    Mcp {
        profile: OsString,
    },
    Help(Option<String>),
}

pub const VERBS: [&str; 6] = ["render", "index", "maps", "profiles", "mcp", "help"];

/// First arguments that mean `help` (compared case-insensitively).
pub const HELP_ALIASES: [&str; 4] = ["help", "--help", "-h", "/?"];

/// Default zoom divisor of `render` (the reference's 1:1).
pub const DEFAULT_ZOOM: u32 = 1;
/// Default gamma of `render` (the reference's 1.30).
pub const DEFAULT_GAMMA: f32 = 1.30;

/// `Ok(None)` when the first argument is not a CLI verb.
pub fn parse(args: &[OsString]) -> Result<Option<Command>, UsageError> {
    let Some(first) = args.first().and_then(|a| a.to_str()) else {
        return Ok(None);
    };
    let verb = first.to_ascii_lowercase();
    let rest = &args[1..];
    if HELP_ALIASES.contains(&verb.as_str()) {
        return parse_help(rest).map(Some);
    }
    // `render --help` (any help alias after a real verb) is that verb's help.
    let is_verb = matches!(
        verb.as_str(),
        "render" | "index" | "maps" | "profiles" | "mcp"
    );
    let asks_help = rest.iter().any(|a| {
        a.to_str()
            .is_some_and(|t| HELP_ALIASES.contains(&t.to_ascii_lowercase().as_str()))
    });
    if is_verb && asks_help {
        return parse_help(&args[..1]).map(Some);
    }
    match verb.as_str() {
        "render" => parse_render(rest).map(|a| Some(Command::Render(a))),
        "index" => parse_index(rest).map(Some),
        "maps" => parse_maps(rest).map(Some),
        "profiles" => parse_profiles(rest).map(Some),
        "mcp" => parse_mcp(rest).map(Some),
        _ => Ok(None),
    }
}

/// Flags (all take a value) and positionals of one verb.
struct Parsed {
    flags: Vec<(String, OsString)>,
    positionals: Vec<OsString>,
}

impl Parsed {
    fn take(&mut self, name: &str) -> Option<OsString> {
        let i = self.flags.iter().position(|(n, _)| n == name)?;
        Some(self.flags.remove(i).1)
    }

    fn take_str(&mut self, verb: &'static str, name: &str) -> Result<Option<String>, UsageError> {
        match self.take(name) {
            None => Ok(None),
            Some(v) => v
                .into_string()
                .map(Some)
                .map_err(|_| usage(verb, format!("--{name} must be valid UTF-8"))),
        }
    }

    fn take_u32(&mut self, verb: &'static str, name: &str) -> Result<Option<u32>, UsageError> {
        match self.take_str(verb, name)? {
            None => Ok(None),
            Some(v) => v.trim().parse::<u32>().map(Some).map_err(|_| {
                usage(
                    verb,
                    format!("--{name} needs a non-negative integer, got {v:?}"),
                )
            }),
        }
    }

    fn require(&mut self, verb: &'static str, name: &str) -> Result<OsString, UsageError> {
        self.take(name)
            .ok_or_else(|| usage(verb, format!("missing --{name}")))
    }
}

fn split(verb: &'static str, args: &[OsString], known: &[&str]) -> Result<Parsed, UsageError> {
    let mut parsed = Parsed {
        flags: Vec::new(),
        positionals: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        let text = arg.to_str();
        let Some(flag) = text.and_then(|t| t.strip_prefix("--")) else {
            parsed.positionals.push(arg.clone());
            continue;
        };
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(OsString::from(v))),
            None => (flag, None),
        };
        let name = name.to_ascii_lowercase();
        if !known.contains(&name.as_str()) {
            return Err(usage(verb, format!("unknown flag --{name}")));
        }
        if parsed.flags.iter().any(|(n, _)| *n == name) {
            return Err(usage(verb, format!("--{name} given twice")));
        }
        let value = match inline {
            Some(v) => v,
            None => {
                let next = args.get(i);
                match next {
                    Some(v) if !v.to_str().is_some_and(|t| t.starts_with("--")) => {
                        i += 1;
                        v.clone()
                    }
                    _ => return Err(usage(verb, format!("--{name} needs a value"))),
                }
            }
        };
        parsed.flags.push((name, value));
    }
    Ok(parsed)
}

/// `1:N` or plain `N`, N in 1..=16.
pub fn parse_zoom(text: &str) -> Option<u32> {
    let t = text.trim();
    let n = t.strip_prefix("1:").unwrap_or(t);
    let n: u32 = n.parse().ok()?;
    (1..=16).contains(&n).then_some(n)
}

fn parse_layers(text: &str) -> Result<Visibility, String> {
    let mut vis = Visibility {
        floors: [false; 2],
        walls: [false; 4],
        shadow: false,
        roofs: false,
        special: false,
        objects: false,
    };
    let mut any = false;
    for item in text.split(',') {
        let item = item.trim().to_ascii_lowercase();
        if item.is_empty() {
            continue;
        }
        any = true;
        match item.as_str() {
            "floors" | "floor" => vis.floors = [true; 2],
            "walls" | "wall" => vis.walls = [true; 4],
            "shadow" | "shadows" => vis.shadow = true,
            "roofs" | "roof" => vis.roofs = true,
            "special" => vis.special = true,
            "objects" | "object" => vis.objects = true,
            "all" => {
                vis = Visibility {
                    special: true,
                    ..Visibility::default()
                }
            }
            other => {
                return Err(format!(
                    "unknown layer {other:?} (floors, walls, shadow, roofs, special, objects, all)"
                ));
            }
        }
    }
    if any {
        Ok(vis)
    } else {
        Err("--layers needs at least one layer".to_owned())
    }
}

fn parse_render(args: &[OsString]) -> Result<RenderArgs, UsageError> {
    const V: &str = "render";
    let mut p = split(
        V,
        args,
        &[
            "profile",
            "lvltype",
            "def",
            "zoom",
            "gamma",
            "layers",
            "variant",
            "object-frame",
            "out",
        ],
    )?;
    let map = match p.positionals.as_slice() {
        [m] => m.clone(),
        [] => return Err(usage(V, "missing <map>")),
        [_, extra, ..] => {
            return Err(usage(
                V,
                format!("unexpected argument {:?}", extra.to_string_lossy()),
            ));
        }
    };
    let profile = p.require(V, "profile")?;
    let out = PathBuf::from(p.require(V, "out")?);
    let lvltype = p.take_u32(V, "lvltype")?;
    let def = p.take_u32(V, "def")?;
    if def.is_some() && lvltype.is_none() {
        return Err(usage(V, "--def needs --lvltype"));
    }
    let zoom = match p.take_str(V, "zoom")? {
        None => DEFAULT_ZOOM,
        Some(z) => parse_zoom(&z)
            .ok_or_else(|| usage(V, format!("bad --zoom {z:?}: use 1:N with N from 1 to 16")))?,
    };
    let gamma = match p.take_str(V, "gamma")? {
        None => DEFAULT_GAMMA,
        Some(g) => g
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|g| g.is_finite() && *g > 0.0 && *g <= 10.0)
            .ok_or_else(|| {
                usage(
                    V,
                    format!("bad --gamma {g:?}: use a number above 0 up to 10"),
                )
            })?,
    };
    let visibility = match p.take_str(V, "layers")? {
        None => Visibility::default(),
        Some(l) => parse_layers(&l).map_err(|m| usage(V, m))?,
    };
    let variant = match p.take_str(V, "variant")? {
        None => VariantRule::default(),
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "reference" => VariantRule::ReferenceQuirk,
            "documented" => VariantRule::Documented,
            _ => {
                return Err(usage(
                    V,
                    format!("bad --variant {v:?}: use reference or documented"),
                ));
            }
        },
    };
    let object_frame = match p.take_str(V, "object-frame")? {
        None => 0,
        Some(f) => f.trim().parse::<u8>().map_err(|_| {
            usage(
                V,
                format!("bad --object-frame {f:?}: use a frame number from 0 to 255"),
            )
        })?,
    };
    Ok(RenderArgs {
        map,
        profile,
        lvltype,
        def,
        zoom,
        gamma,
        visibility,
        variant,
        object_frame,
        out,
    })
}

fn parse_index(args: &[OsString]) -> Result<Command, UsageError> {
    const V: &str = "index";
    let mut p = split(V, args, &["profile"])?;
    let sub = match p.positionals.as_slice() {
        [s] => s.to_str().map(str::to_ascii_lowercase),
        [] => return Err(usage(V, "missing subcommand (status or build)")),
        [_, extra, ..] => {
            return Err(usage(
                V,
                format!("unexpected argument {:?}", extra.to_string_lossy()),
            ));
        }
    };
    let which = match sub.as_deref() {
        Some("status") => false,
        Some("build") => true,
        _ => return Err(usage(V, "unknown subcommand (status or build)")),
    };
    let profile = p.require(V, "profile")?;
    Ok(if which {
        Command::IndexBuild { profile }
    } else {
        Command::IndexStatus { profile }
    })
}

fn parse_maps(args: &[OsString]) -> Result<Command, UsageError> {
    const V: &str = "maps";
    let mut p = split(V, args, &["profile", "filter"])?;
    match p.positionals.as_slice() {
        [s] if s.to_str().is_some_and(|s| s.eq_ignore_ascii_case("list")) => {}
        [] => return Err(usage(V, "missing subcommand (list)")),
        [s] => {
            return Err(usage(
                V,
                format!("unknown subcommand {:?} (list)", s.to_string_lossy()),
            ));
        }
        [_, extra, ..] => {
            return Err(usage(
                V,
                format!("unexpected argument {:?}", extra.to_string_lossy()),
            ));
        }
    }
    let profile = p.require(V, "profile")?;
    let filter = p.take_str(V, "filter")?;
    Ok(Command::MapsList { profile, filter })
}

fn parse_profiles(args: &[OsString]) -> Result<Command, UsageError> {
    const V: &str = "profiles";
    let p = split(V, args, &[])?;
    match p.positionals.as_slice() {
        [s] if s.to_str().is_some_and(|s| s.eq_ignore_ascii_case("list")) => {
            Ok(Command::ProfilesList)
        }
        [] => Err(usage(V, "missing subcommand (list)")),
        [s, ..] => Err(usage(
            V,
            format!("unknown subcommand {:?} (list)", s.to_string_lossy()),
        )),
    }
}

fn parse_help(args: &[OsString]) -> Result<Command, UsageError> {
    const V: &str = "help";
    let p = split(V, args, &[])?;
    match p.positionals.as_slice() {
        [] => Ok(Command::Help(None)),
        [v] => Ok(Command::Help(Some(
            v.to_string_lossy().to_ascii_lowercase(),
        ))),
        [_, extra, ..] => Err(usage(
            V,
            format!("unexpected argument {:?}", extra.to_string_lossy()),
        )),
    }
}

fn parse_mcp(args: &[OsString]) -> Result<Command, UsageError> {
    const V: &str = "mcp";
    let mut p = split(V, args, &["profile"])?;
    if let Some(extra) = p.positionals.first() {
        return Err(usage(
            V,
            format!("unexpected argument {:?}", extra.to_string_lossy()),
        ));
    }
    let profile = p.require(V, "profile")?;
    Ok(Command::Mcp { profile })
}
