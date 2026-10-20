//! Reading Diablo II's crash logs (`D2YYMMDD.txt` in the game's folder) and
//! explaining the crashes a map can cause, in plain words.
//!
//! The game appends to the day's log on every start and every crash. A crash
//! block has a `PROGRAM:` header, then either `***** UNHANDLED EXCEPTION`
//! with the fault address and module, or a `Halt` (an assertion) with its line
//! and address, then a list of the loaded modules. `Error opening file` lines
//! just before it name what was missing.

/// A module in a crash block's module list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrashModule {
    /// File name, e.g. `D2Client.dll`.
    pub name: String,
    /// Load address.
    pub base: u64,
    /// Size in bytes.
    pub size: u64,
}

/// Exception or assertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashKind {
    /// `***** UNHANDLED EXCEPTION`.
    Exception,
    /// A `Halt` (failed internal check).
    Halt,
}

/// One crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crash {
    /// "2026-09-27 10:30:29" (from the block's TIME line); empty if absent.
    pub time: String,
    /// Exception or halt.
    pub kind: CrashKind,
    /// Exception name ("ACCESS_VIOLATION") or the halt's expression.
    pub what: String,
    /// Module the crash happened in, when known.
    pub module: Option<String>,
    /// Offset in that module, when known.
    pub offset: Option<u64>,
    /// The halt's source line.
    pub line: Option<u32>,
    /// Address of the crash (a halt's comes from its expression).
    pub address: Option<u64>,
    /// Files the game failed to open in the moments before (this session,
    /// most recent last, at most 20).
    pub failed_files: Vec<String>,
}

fn strip_time(raw: &str) -> &str {
    let b = raw.as_bytes();
    let digits = |r: std::ops::Range<usize>| {
        r.into_iter()
            .all(|i| b.get(i).is_some_and(u8::is_ascii_digit))
    };
    if b.len() > 9
        && digits(0..2)
        && b[2] == b':'
        && digits(3..5)
        && b[5] == b':'
        && digits(6..8)
        && b[8] == b'.'
    {
        let mut i = 9;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i > 9 && b.get(i).is_some_and(u8::is_ascii_whitespace) {
            return raw[i..].trim_start();
        }
    }
    raw
}

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s, 16).ok()
}

fn module_in(line: &str) -> Option<CrashModule> {
    let p = line.find("Base:")?;
    let rest = &line[p + 5..];
    let h = rest.find(['h', 'H'])?;
    let base = parse_hex(&rest[..h])?;
    let rest = rest[h + 1..]
        .trim_start()
        .strip_prefix("Size:")?
        .trim_start();
    let h = rest.find(['h', 'H'])?;
    let size = parse_hex(&rest[..h])?;
    let rest = rest[h + 1..].trim_start().strip_prefix("Name:")?;
    let name = rest.split_whitespace().next()?;
    Some(CrashModule {
        name: name.to_string(),
        base,
        size,
    })
}

fn trailing_address(s: &str) -> Option<u64> {
    let t = s.trim_end();
    let run = t.bytes().rev().take_while(u8::is_ascii_hexdigit).count();
    if run < 6 {
        return None;
    }
    parse_hex(&t[t.len() - run.min(8)..])
}

/// Every crash in a log, oldest first.
pub fn parse_crash_log(text: &str) -> Vec<Crash> {
    let mut out = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut cur: Option<Crash> = None;
    let mut modules: Vec<CrashModule> = Vec::new();
    let finish = |cur: &mut Option<Crash>, modules: &mut Vec<CrashModule>, out: &mut Vec<Crash>| {
        if let Some(mut c) = cur.take() {
            if let (Some(a), None) = (c.address, &c.module)
                && let Some(m) = modules.iter().find(|m| a >= m.base && a < m.base + m.size)
            {
                c.module = Some(m.name.clone());
                c.offset = Some(a - m.base);
            }
            if !c.what.is_empty() || c.module.is_some() {
                out.push(c);
            }
        }
        modules.clear();
    };
    for raw in text.lines() {
        let line = strip_time(raw);
        if line.starts_with("SE Realm startup at") || line.contains("Diablo II running under") {
            finish(&mut cur, &mut modules, &mut out);
            failed.clear();
            continue;
        }
        if let Some(f) = line.strip_prefix("Error opening file:") {
            failed.push(f.trim().to_string());
            if failed.len() > 20 {
                failed.remove(0);
            }
            continue;
        }
        if line.starts_with("PROGRAM:") {
            finish(&mut cur, &mut modules, &mut out);
            cur = Some(Crash {
                time: String::new(),
                kind: CrashKind::Exception,
                what: String::new(),
                module: None,
                offset: None,
                line: None,
                address: None,
                failed_files: std::mem::take(&mut failed),
            });
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        if let Some(t) = line.strip_prefix("TIME:") {
            let t = t.trim_start();
            if t.len() >= 19 && t.is_char_boundary(19) {
                let s = &t[..19];
                let b = s.as_bytes();
                let ok = b.iter().enumerate().all(|(i, &ch)| match i {
                    4 | 7 => ch == b'-',
                    10 => ch == b' ',
                    13 | 16 => ch == b':',
                    _ => ch.is_ascii_digit(),
                });
                if ok {
                    c.time = s.to_string();
                }
            }
        }
        if let Some(rest) = line.strip_prefix('*')
            && let Some(e) = rest
                .trim_start_matches('*')
                .trim_start()
                .strip_prefix("UNHANDLED EXCEPTION:")
        {
            c.kind = CrashKind::Exception;
            c.what = e.split_whitespace().next().unwrap_or("").to_string();
        }
        if let Some(f) = line.strip_prefix("Fault address:") {
            let mut it = f.split_whitespace();
            if let (Some(a), Some(sec)) = (it.next(), it.next()) {
                let file = it.collect::<Vec<_>>().join(" ");
                c.address = parse_hex(a);
                if let Some((section, off)) = sec.split_once(':')
                    && !file.is_empty()
                    && !file.to_lowercase().contains("unknown")
                    && section != "00"
                {
                    c.module = file
                        .rsplit(['\\', '/'])
                        .next()
                        .map(|s| s.trim().to_string());
                    // The offset is within section 01 (.text), which starts at 0x1000 in these DLLs.
                    c.offset = parse_hex(off).map(|o| o + if section == "01" { 0x1000 } else { 0 });
                }
            }
        }
        if line
            .strip_prefix("Halt")
            .is_some_and(|r| !r.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_'))
        {
            c.kind = CrashKind::Halt;
        }
        if let Some(l) = line.strip_prefix("Location")
            && let Some(l) = l.trim_start().strip_prefix(':')
            && let Some(p) = l.find("line #")
        {
            let digits: String = l[p + 6..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(n) = digits.parse() {
                c.kind = CrashKind::Halt;
                c.line = Some(n);
            }
        }
        if let Some(e) = line.strip_prefix("Expression")
            && let Some(e) = e.trim_start().strip_prefix(':')
            && !e.trim().is_empty()
        {
            c.what = e.trim().to_string();
            if let Some(a) = trailing_address(&c.what) {
                c.address = Some(a);
            }
        }
        if let Some(m) = module_in(line) {
            modules.push(m);
        }
    }
    finish(&mut cur, &mut modules, &mut out);
    out
}

/// What recognised the crash (for the caller to branch on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// A level's loading-screen image (Levels.txt EntryFile) is missing.
    EntryImage,
    /// A level sits too far out in its act (automap positions overflow).
    AutomapRange,
    /// AutoMap.txt has a rule whose Cel1 is -1.
    AutomapCel,
    /// A preset level's DS1 is not the size Levels.txt gives it.
    PresetSize,
    /// LvlPrest AutoMap is 1 on a level that is not a town.
    AutomapReveal,
    /// A string table's checksum is wrong.
    StringTable,
    /// Some other file the game could not open.
    MissingFile,
    /// Not recognised; only the facts are shown.
    Unknown,
}

/// Names the levels whose `EntryFile` is a given image name.
pub type LevelsWithEntry<'a> = dyn Fn(&str) -> Vec<String> + 'a;

/// A plain-language reading of a crash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explanation {
    /// One line: what happened.
    pub title: String,
    /// What to do about it.
    pub detail: String,
    /// Whether the crash is recognised.
    pub known: bool,
    /// Running the compatibility check on the map finds and fixes it.
    pub check: bool,
    /// What it was.
    pub cause: Cause,
}

fn hex(n: Option<u64>) -> String {
    n.map_or(String::new(), |n| format!("0x{n:x}"))
}

/// What a crash means for a map maker. `levels_with_entry` names the levels
/// whose EntryFile is a given image name.
pub fn explain_crash(c: &Crash, levels_with_entry: Option<&LevelsWithEntry<'_>>) -> Explanation {
    let where_ = match (&c.module, c.address) {
        (None, Some(a)) => format!("at address {}, outside the game's code", hex(Some(a))),
        _ => format!(
            "in {}{}{}",
            c.module.as_deref().unwrap_or("an unknown module"),
            c.offset
                .map_or(String::new(), |o| format!(" +{}", hex(Some(o)))),
            c.line.map_or(String::new(), |l| format!(", line {l}"))
        ),
    };
    let last_failed = c.failed_files.last();
    let in_range = |lo: u64, hi: u64| c.address.is_some_and(|a| a >= lo && a <= hi);
    let module_lc = c.module.as_deref().unwrap_or("").to_lowercase();

    // A loading-screen image the game couldn't open: it reads it anyway and crashes in D2CMP.
    if let Some(f) = last_failed {
        let up = f.to_lowercase().replace('/', "\\");
        if let Some(p) = up.rfind("\\expansion\\")
            && up.ends_with(".dc6")
            && !up[p + 11..].contains('\\')
        {
            let name = &f[p + 11..f.len() - 4];
            let levels = if name.is_empty() {
                Vec::new()
            } else {
                levels_with_entry.map_or_else(Vec::new, |f| f(name))
            };
            let title = if name.is_empty() {
                "A level's loading-screen image (EntryFile) is empty".to_string()
            } else {
                format!("The loading-screen image \"{name}.dc6\" doesn't exist")
            };
            let which = if name.is_empty() {
                "an image with no name".to_string()
            } else {
                format!("\"{name}.dc6\"")
            };
            let used = if levels.is_empty() {
                String::new()
            } else {
                format!(" Levels using it: {}.", levels.join(", "))
            };
            return Explanation {
                known: true,
                check: true,
                cause: Cause::EntryImage,
                title,
                detail: format!(
                    "Levels.txt's EntryFile names the picture shown while a level loads (data/local/ui/eng/expansion/<EntryFile>.dc6). The game couldn't open {which} and crashed on the loading screen.{used} Set EntryFile to an image the game has (Harrogath's is A5L1); the compatibility check offers the fix."
                ),
            };
        }
    }
    let halt = c.kind == CrashKind::Halt;
    // D2Client's automap marker range check: the level is placed too far out in the act's world.
    if halt
        && (in_range(0x6fb1_17a0, 0x6fb1_1840)
            || (module_lc.contains("d2client") && c.line.is_some_and(|l| (965..=967).contains(&l))))
    {
        return Explanation {
            known: true,
            check: true,
            cause: Cause::AutomapRange,
            title: "A level is placed too far out in its act (automap positions overflow)".into(),
            detail: "The automap keeps positions in 16 bits: a level whose Levels.txt OffsetX - OffsetY or OffsetX + OffsetY is too large makes the game stop as soon as a player, NPC or object is put on the automap (usually a few steps in). The compatibility check finds the level and moves it to a free spot inside the range.".into(),
        };
    }
    // The AutoMap loader asserts when Cel1 is the -1 terminator.
    if halt && c.line == Some(1310) && c.address == Some(0x6fdb_d027) {
        return Explanation {
            known: true,
            check: true,
            cause: Cause::AutomapCel,
            title: "AutoMap.txt has a rule without a valid picture".into(),
            detail: "The game stops loading when an AutoMap.txt row starts with Cel1 = -1. Remove the empty matching rule (keeping other sequences), then rebuild AutoMap.bin from the corrected table. A tile with no matching rule stays off the automap safely. Back up both files before repairing them.".into(),
        };
    }
    // D2Common building a preset level whose DS1 isn't the size Levels.txt gives the level.
    let in_common = |lo: u64, hi: u64| in_range(lo, hi);
    if halt
        && (in_common(0x6fd5_c060, 0x6fd5_c680)
            || (matches!(c.line, Some(2239 | 2240))
                && (c.address.is_none() || in_common(0x6fd5_0000, 0x6fdf_9000))))
    {
        return Explanation {
            known: true,
            check: true,
            cause: Cause::PresetSize,
            title: "A preset level's map isn't the size the level expects".into(),
            detail: format!(
                "The game checks, as it builds a preset level (entering it, or opening a portal to it), that the DS1's {} matches the level's Levels.txt SizeX/SizeY (the DS1 size minus one, all difficulties). This happens after resizing a map without updating the level. Open the map and save it to update the level size, or run the compatibility check and apply its fix.",
                if c.line == Some(2240) {
                    "height"
                } else {
                    "width"
                }
            ),
        };
    }
    // D2Client revealing the whole automap of a level whose LvlPrest AutoMap is 1 but that isn't a town.
    if halt && in_range(0x6fb1_1900, 0x6fb1_1a40) {
        return Explanation {
            known: true,
            check: true,
            cause: Cause::AutomapReveal,
            title: "A level reveals its whole automap on entry (LvlPrest AutoMap is 1) but isn't a town".into(),
            detail: "Only the game's five towns may have AutoMap 1 in LvlPrest.txt; for any other level the game stops while entering it. Set AutoMap to 0 (the compatibility check offers it).".into(),
        };
    }
    // D2Lang stops at start-up when a string table's checksum doesn't match.
    if module_lc.contains("d2lang")
        || (halt && c.line == Some(208) && c.address.is_some_and(|a| a < 0x0100_0000))
    {
        return Explanation {
            known: true,
            check: false,
            cause: Cause::StringTable,
            title: "A string table (.tbl) is damaged or its checksum is wrong".into(),
            detail: "The game checks each .tbl in data/local/lng when it starts; one written by a tool that didn't update its checksum stops it. Save the table again with a tool that writes the checksum, or restore its .bak.".into(),
        };
    }
    // Any other file the game couldn't open just before crashing.
    if let Some(lf) = last_failed {
        let f = lf.replace('\\', "/");
        let lower = f.to_lowercase();
        let (t, d) = if lower.ends_with(".dt1") {
            (
                "tile library (DT1)",
                "A level type in LvlTypes.txt lists it and the map's Dt1Mask selects it. Put the DT1 there, or take it out of the level type.",
            )
        } else if lower.ends_with(".ds1") {
            (
                "map (DS1)",
                "LvlPrest.txt points at it (File1-File6). Check the path and that the file is in your mod.",
            )
        } else if lower.ends_with(".dcc")
            || lower.ends_with(".cof")
            || ["/objects/", "/monsters/", "/chars/"]
                .iter()
                .any(|d| lower.contains(d))
        {
            (
                "sprite",
                "An object, NPC or monster there uses graphics that aren't in the game or your mod.",
            )
        } else if lower.ends_with(".tbl") {
            (
                "string table",
                "Check the file exists in data/local/lng/<language>.",
            )
        } else {
            (
                "file",
                "Check the path exists in your mod or the game's archives.",
            )
        };
        let base = f.rsplit('/').next().unwrap_or(&f);
        return Explanation {
            known: true,
            check: lower.ends_with(".dt1") || lower.ends_with(".ds1"),
            cause: Cause::MissingFile,
            title: format!("The game couldn't open a {t}: {base}"),
            detail: format!("{f}. {d} (The crash itself was {where_}.)"),
        };
    }
    let what = if halt {
        "The game stopped on an internal check".to_string()
    } else {
        format!(
            "The game crashed ({})",
            if c.what.is_empty() {
                "unknown error"
            } else {
                &c.what
            }
        )
    };
    Explanation {
        known: false,
        check: true,
        cause: Cause::Unknown,
        title: format!("{what} {where_}"),
        detail: format!(
            "This crash isn't recognised.{} If it happens when entering a map you made, run the compatibility check on that map; if not, it may be the game or another mod.",
            if halt && !c.what.is_empty() {
                format!(" \"{}\".", c.what)
            } else {
                String::new()
            }
        ),
    }
}
