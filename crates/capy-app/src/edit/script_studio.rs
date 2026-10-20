//! `CAPY_SCRIPT` commands of the ds1-studio parity tools (P9/P10). The
//! runner is `crate::ui::studio::script`.
//!
//! ```text
//! set-brush floor0 0x017000c2 [orientation]   # the value Paint writes
//! mix-add floor0 0x017100c2                    # Ctrl+click a tile into the mix
//! paint-rect floor0 2 2 6 5 [erase]            # Rectangle paint / erase
//! fill floor0 3 3 [erase]                      # Fill area (connected, same tile)
//! replace 0/5/2 0/7/0                          # Find & replace (o/m/s keys)
//! new-map data\global\tiles\act1\town\mine.ds1 20 16 1
//! drag-edge right 3                            # drag a map edge 3 cells out
//! view walk                                    # tiles objects walk automap light roofs
//! assert-view walk
//! assert-tile floor0 3 4 5 2                   # main and sub; `empty` for none
//! assert-size 40 32
//! assert-walkable 3512.5 20                    # badge value, tolerance
//! export-png out.png [selection]
//! copy-view                                    # PrintScreen, waits for the capture
//! ```

use std::path::PathBuf;

use capy_edit::LayerRef;

use super::script::parse_layer;

#[derive(Clone, Debug, PartialEq)]
pub enum StudioCmd {
    SetBrush {
        layer: LayerRef,
        cell: u32,
        orientation: u32,
    },
    MixAdd {
        layer: LayerRef,
        cell: u32,
        orientation: u32,
    },
    PaintRect {
        layer: LayerRef,
        x0: u32,
        y0: u32,
        x1: u32,
        y1: u32,
        erase: bool,
    },
    Fill {
        layer: LayerRef,
        x: u32,
        y: u32,
        erase: bool,
    },
    Replace {
        from: String,
        to: String,
    },
    NewMap {
        path: String,
        width: u32,
        height: u32,
        act: u8,
    },
    DragEdge {
        edge: String,
        cells: i32,
    },
    View(String),
    AssertView(String),
    AssertTile {
        layer: LayerRef,
        x: u32,
        y: u32,
        /// `None`: the cell must be empty.
        tile: Option<(u8, u8)>,
    },
    AssertSize(u32, u32),
    AssertWalkable {
        expected: f64,
        tolerance: f64,
    },
    ExportPng {
        file: PathBuf,
        selection: bool,
    },
    CopyView,
}

fn num<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, String> {
    let t = text.trim();
    let parsed = if let Some(hex) = t.strip_prefix("0x") {
        u64::from_str_radix(hex, 16)
            .ok()
            .and_then(|v| v.to_string().parse().ok())
    } else {
        t.parse().ok()
    };
    parsed.ok_or_else(|| format!("{what}: {text:?} is not a number"))
}

/// `None`: not one of these words.
pub fn parse(word: &str, args: &[&str], rest: &str) -> Option<Result<StudioCmd, String>> {
    let need = |n: std::ops::RangeInclusive<usize>, usage: &str| -> Result<(), String> {
        if n.contains(&args.len()) {
            Ok(())
        } else {
            Err(format!("{word}: expected {usage}"))
        }
    };
    let flag = |i: usize, name: &str| args.get(i).is_some_and(|a| a.eq_ignore_ascii_case(name));
    let r = (|| -> Result<StudioCmd, String> {
        Ok(match word {
            "set-brush" | "mix-add" => {
                need(2..=3, "<layer> <cell hex> [orientation]")?;
                let layer = parse_layer(args[0])?;
                let cell = num(args[1], "cell")?;
                let orientation = args.get(2).map_or(Ok(0), |o| num(o, "orientation"))?;
                if word == "set-brush" {
                    StudioCmd::SetBrush {
                        layer,
                        cell,
                        orientation,
                    }
                } else {
                    StudioCmd::MixAdd {
                        layer,
                        cell,
                        orientation,
                    }
                }
            }
            "paint-rect" => {
                need(5..=6, "<layer> x0 y0 x1 y1 [erase]")?;
                StudioCmd::PaintRect {
                    layer: parse_layer(args[0])?,
                    x0: num(args[1], "x0")?,
                    y0: num(args[2], "y0")?,
                    x1: num(args[3], "x1")?,
                    y1: num(args[4], "y1")?,
                    erase: flag(5, "erase"),
                }
            }
            "fill" => {
                need(3..=4, "<layer> x y [erase]")?;
                StudioCmd::Fill {
                    layer: parse_layer(args[0])?,
                    x: num(args[1], "x")?,
                    y: num(args[2], "y")?,
                    erase: flag(3, "erase"),
                }
            }
            "replace" => {
                need(2..=2, "<o/m/s> <o/m/s>")?;
                StudioCmd::Replace {
                    from: args[0].to_owned(),
                    to: args[1].to_owned(),
                }
            }
            "new-map" => {
                need(4..=4, "<game path> <width> <height> <act 1-5>")?;
                StudioCmd::NewMap {
                    path: args[0].to_owned(),
                    width: num(args[1], "width")?,
                    height: num(args[2], "height")?,
                    act: num(args[3], "act")?,
                }
            }
            "drag-edge" => {
                need(2..=2, "left|top|right|bottom <cells>")?;
                StudioCmd::DragEdge {
                    edge: args[0].to_ascii_lowercase(),
                    cells: num(args[1], "cells")?,
                }
            }
            "view" => {
                need(1..=1, "tiles|objects|walk|automap|light|roofs")?;
                StudioCmd::View(args[0].to_ascii_lowercase())
            }
            "assert-view" => {
                need(1..=1, "tiles|objects|walk|automap|light|roofs")?;
                StudioCmd::AssertView(args[0].to_ascii_lowercase())
            }
            "assert-tile" => {
                need(4..=5, "<layer> x y <main> <sub> | empty")?;
                let tile = if flag(3, "empty") {
                    None
                } else {
                    need(5..=5, "<layer> x y <main> <sub>")?;
                    Some((num(args[3], "main")?, num(args[4], "sub")?))
                };
                StudioCmd::AssertTile {
                    layer: parse_layer(args[0])?,
                    x: num(args[1], "x")?,
                    y: num(args[2], "y")?,
                    tile,
                }
            }
            "assert-size" => {
                need(2..=2, "<width> <height>")?;
                StudioCmd::AssertSize(num(args[0], "width")?, num(args[1], "height")?)
            }
            "assert-walkable" => {
                need(2..=2, "<expected> <tolerance>")?;
                StudioCmd::AssertWalkable {
                    expected: num(args[0], "expected")?,
                    tolerance: num(args[1], "tolerance")?,
                }
            }
            "export-png" => {
                if rest.is_empty() {
                    return Err("export-png: expected <file> [selection]".to_owned());
                }
                let (file, selection) = match rest.rsplit_once(char::is_whitespace) {
                    Some((f, s)) if s.eq_ignore_ascii_case("selection") => (f.trim(), true),
                    _ => (rest, false),
                };
                StudioCmd::ExportPng {
                    file: PathBuf::from(file),
                    selection,
                }
            }
            "copy-view" => StudioCmd::CopyView,
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

    fn p(line: &str) -> Option<Result<StudioCmd, String>> {
        let (w, rest) = line.split_once(' ').unwrap_or((line, ""));
        let args: Vec<&str> = rest.split_whitespace().collect();
        parse(w, &args, rest)
    }

    #[test]
    fn commands_parse() {
        assert_eq!(
            p("paint-rect floor0 1 2 3 4 erase"),
            Some(Ok(StudioCmd::PaintRect {
                layer: LayerRef::Floor(0),
                x0: 1,
                y0: 2,
                x1: 3,
                y1: 4,
                erase: true
            }))
        );
        assert_eq!(
            p("set-brush wall0 0x00100281 3"),
            Some(Ok(StudioCmd::SetBrush {
                layer: LayerRef::Wall(0),
                cell: 0x0010_0281,
                orientation: 3
            }))
        );
        assert_eq!(
            p("assert-tile floor0 3 4 empty"),
            Some(Ok(StudioCmd::AssertTile {
                layer: LayerRef::Floor(0),
                x: 3,
                y: 4,
                tile: None
            }))
        );
        assert_eq!(
            p("export-png C:\\a b\\out.png selection"),
            Some(Ok(StudioCmd::ExportPng {
                file: PathBuf::from("C:\\a b\\out.png"),
                selection: true
            }))
        );
        assert_eq!(p("drag-edge right x").map(|r| r.is_err()), Some(true));
        assert_eq!(p("unknown-thing 1"), None);
    }
}
