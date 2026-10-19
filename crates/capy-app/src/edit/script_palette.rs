//! `CAPY_SCRIPT` commands for "Convert to palette" and palette memory
//! (parser; the runner is `crate::script_run::palette_step`).
//!
//! ```text
//! convert-open data\global\tiles\act5\lava\lavafloor.dt1   # or a file path, or `viewer`
//! convert-source act5                   # palette labels: act1..act5, units, menu0, ...
//! convert-target act1                   # or: all-acts
//! convert-setting distance redmean      # oklab | redmean | rgb
//! convert-setting targets shared        # all | shared | range <from> <to>
//! convert-setting keep-exact off
//! convert-setting lightness 1.5         # also brightness, contrast, saturation, hue
//! convert-setting reset
//! convert-setting preset-save <name>    # or preset-load <name>
//! convert-override 239 164              # source target; `convert-override 239 clear`,
//!                                       # `convert-override clear` (all)
//! convert-item 12                       # the frame or tile on show
//! convert-tile 0 3 1 [n]                # a DT1 tile by orientation main sub
//! convert-picker 239                    # open the override picker (or: off)
//! convert-hover 239                     # highlight a row's pixels (or: off)
//! convert-highlight changed             # changed | worst | off
//! convert-apply                         # the Convert button
//! convert-save-game [game path]         # default: where the file came from
//! convert-save-as C:\out\a.dt1
//! convert-close
//! convert-batch-add-game tiles\act5\lava
//! convert-batch-add-folder C:\in
//! convert-batch-output saves            # or a folder
//! convert-batch-start                   # waits until the batch is done
//! palette act5                          # pick a palette for the Viewer's file
//! assert-palette-badge act5 remembered [viewer|editor|converter]
//!                                       # remembered | remembered-other | folder |
//!                                       # folder-other | none
//! sprite-save-game-checked <game path>  # like sprite-save-game, with the palette question
//! palette-warning use                   # use | keep | cancel (fails when none is open)
//! ```

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum PaletteCmd {
    Open(String),
    Source(String),
    Target(String),
    Setting {
        key: String,
        value: Vec<String>,
    },
    Override {
        source: Option<u8>,
        target: Option<u8>,
    },
    Item(usize),
    Tile {
        o: i32,
        m: i32,
        s: i32,
        n: Option<u32>,
    },
    Picker(Option<u8>),
    Hover(Option<u8>),
    Highlight(String),
    Apply,
    SaveGame(Option<String>),
    SaveAs(PathBuf),
    Close,
    BatchAddGame(String),
    BatchAddFolder(PathBuf),
    /// `None` = saves.
    BatchOutput(Option<PathBuf>),
    BatchStart,
    Pick(String),
    AssertBadge {
        label: String,
        source: String,
        panel: String,
    },
    SaveChecked(String),
    Warning(String),
}

fn index(text: &str) -> Result<u8, String> {
    text.trim()
        .parse::<u8>()
        .map_err(|_| format!("{text:?} is not a palette index 0..255"))
}

fn num<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{what}: {text:?} is not a number"))
}

fn off_or_index(text: &str) -> Result<Option<u8>, String> {
    if text.eq_ignore_ascii_case("off") {
        Ok(None)
    } else {
        index(text).map(Some)
    }
}

/// `None` when `word` is not one of these commands.
pub fn parse(word: &str, args: &[&str], rest: &str) -> Option<Result<PaletteCmd, String>> {
    let need = |n: usize, usage: &str| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(format!("{word}: expected {usage}"))
        }
    };
    let text = |usage: &str| -> Result<String, String> {
        if rest.is_empty() {
            Err(format!("{word}: expected {usage}"))
        } else {
            Ok(rest.to_owned())
        }
    };
    let r = (|| -> Result<PaletteCmd, String> {
        Ok(match word {
            "convert-open" => PaletteCmd::Open(text("<game path>|<file>|viewer")?),
            "convert-source" => {
                need(1, "<palette>")?;
                PaletteCmd::Source(args[0].to_owned())
            }
            "convert-target" => {
                need(1, "<palette>|all-acts")?;
                PaletteCmd::Target(args[0].to_owned())
            }
            "convert-setting" => {
                let (k, v) = args
                    .split_first()
                    .ok_or("convert-setting: expected <key> [value]")?;
                PaletteCmd::Setting {
                    key: k.to_ascii_lowercase(),
                    value: v.iter().map(|s| (*s).to_owned()).collect(),
                }
            }
            "convert-override" => match args {
                [a] if a.eq_ignore_ascii_case("clear") => PaletteCmd::Override {
                    source: None,
                    target: None,
                },
                [s, t] if t.eq_ignore_ascii_case("clear") => PaletteCmd::Override {
                    source: Some(index(s)?),
                    target: None,
                },
                [s, t] => PaletteCmd::Override {
                    source: Some(index(s)?),
                    target: Some(index(t)?),
                },
                _ => {
                    return Err(
                        "convert-override: expected <source> <target>|clear, or clear".into(),
                    );
                }
            },
            "convert-item" => {
                need(1, "<index>")?;
                PaletteCmd::Item(num(args[0], "item")?)
            }
            "convert-tile" => {
                if !(3..=4).contains(&args.len()) {
                    return Err("convert-tile: expected <orientation> <main> <sub> [n]".into());
                }
                PaletteCmd::Tile {
                    o: num(args[0], "orientation")?,
                    m: num(args[1], "main")?,
                    s: num(args[2], "sub")?,
                    n: args.get(3).map(|n| num(n, "n")).transpose()?,
                }
            }
            "convert-picker" => {
                need(1, "<source>|off")?;
                PaletteCmd::Picker(off_or_index(args[0])?)
            }
            "convert-hover" => {
                need(1, "<source>|off")?;
                PaletteCmd::Hover(off_or_index(args[0])?)
            }
            "convert-highlight" => {
                need(1, "changed|worst|off")?;
                let h = args[0].to_ascii_lowercase();
                if !["changed", "worst", "off"].contains(&h.as_str()) {
                    return Err(format!(
                        "convert-highlight {h:?} must be changed, worst or off"
                    ));
                }
                PaletteCmd::Highlight(h)
            }
            "convert-apply" => {
                need(0, "no arguments")?;
                PaletteCmd::Apply
            }
            "convert-save-game" => {
                PaletteCmd::SaveGame((!rest.is_empty()).then(|| rest.to_owned()))
            }
            "convert-save-as" => PaletteCmd::SaveAs(PathBuf::from(text("<file>")?)),
            "convert-close" => {
                need(0, "no arguments")?;
                PaletteCmd::Close
            }
            "convert-batch-add-game" => PaletteCmd::BatchAddGame(text("<filter>")?),
            "convert-batch-add-folder" => {
                PaletteCmd::BatchAddFolder(PathBuf::from(text("<folder>")?))
            }
            "convert-batch-output" => {
                let t = text("saves|<folder>")?;
                PaletteCmd::BatchOutput(
                    (!t.eq_ignore_ascii_case("saves")).then(|| PathBuf::from(t)),
                )
            }
            "convert-batch-start" => {
                need(0, "no arguments")?;
                PaletteCmd::BatchStart
            }
            "palette" => {
                need(1, "<palette>")?;
                PaletteCmd::Pick(args[0].to_owned())
            }
            "assert-palette-badge" => {
                if !(2..=3).contains(&args.len()) {
                    return Err(
                        "assert-palette-badge: expected <palette> <source> [viewer|editor|converter]"
                            .into(),
                    );
                }
                let panel = args.get(2).map_or("viewer", |p| p).to_ascii_lowercase();
                if !["viewer", "editor", "converter"].contains(&panel.as_str()) {
                    return Err(format!("assert-palette-badge: unknown panel {panel:?}"));
                }
                PaletteCmd::AssertBadge {
                    label: args[0].to_owned(),
                    source: args[1].to_ascii_lowercase(),
                    panel,
                }
            }
            "sprite-save-game-checked" => {
                need(1, "<game path>")?;
                PaletteCmd::SaveChecked(args[0].to_owned())
            }
            "palette-warning" => {
                need(1, "use|keep|cancel")?;
                let a = args[0].to_ascii_lowercase();
                if !["use", "keep", "cancel"].contains(&a.as_str()) {
                    return Err(format!("palette-warning {a:?} must be use, keep or cancel"));
                }
                PaletteCmd::Warning(a)
            }
            _ => return Err(String::new()),
        })
    })();
    match r {
        Err(e) if e.is_empty() => None,
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(line: &str) -> Result<PaletteCmd, String> {
        let (w, rest) = line.split_once(' ').unwrap_or((line, ""));
        let args: Vec<&str> = rest.split_whitespace().collect();
        parse(w, &args, rest.trim()).unwrap_or_else(|| Err("not a palette command".into()))
    }

    #[test]
    fn palette_commands_parse() {
        assert_eq!(
            p("convert-open data\\a b\\x.dt1"),
            Ok(PaletteCmd::Open("data\\a b\\x.dt1".into()))
        );
        assert_eq!(
            p("convert-override 239 164"),
            Ok(PaletteCmd::Override {
                source: Some(239),
                target: Some(164)
            })
        );
        assert_eq!(
            p("convert-override clear"),
            Ok(PaletteCmd::Override {
                source: None,
                target: None
            })
        );
        assert!(p("convert-override 300 1").is_err());
        assert_eq!(
            p("convert-tile 0 3 1"),
            Ok(PaletteCmd::Tile {
                o: 0,
                m: 3,
                s: 1,
                n: None
            })
        );
        assert_eq!(p("convert-save-game"), Ok(PaletteCmd::SaveGame(None)));
        assert_eq!(
            p("convert-batch-output saves"),
            Ok(PaletteCmd::BatchOutput(None))
        );
        assert_eq!(
            p("assert-palette-badge act5 Remembered"),
            Ok(PaletteCmd::AssertBadge {
                label: "act5".into(),
                source: "remembered".into(),
                panel: "viewer".into()
            })
        );
        assert!(p("assert-palette-badge act5 x panel").is_err());
        assert!(p("palette-warning maybe").is_err());
        assert!(p("convert-highlight all").is_err());
        assert_eq!(p("convert-hover off"), Ok(PaletteCmd::Hover(None)));
        assert!(
            parse("open", &[], "").is_none(),
            "other commands pass through"
        );
    }
}
