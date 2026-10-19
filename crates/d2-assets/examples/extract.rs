//! Extract the winning version of every named file of a profile to a folder.
//!
//! ```text
//! cargo run -p d2-assets --release --example extract -- \
//!     --profile vanilla|mxl --out <dir> [--ext ds1,dt1,txt,bin,dat,pl2,tbl,cof,d2]
//! ```
//!
//! Game directories come from `CAPY_VANILLA_DIR` / `CAPY_MXL_DIR` (defaults:
//! the standard install paths). Names are gathered from every archive's
//! listfile; for each `data\global\excel\X.txt` the sibling `X.bin` is also
//! tried (archives without a listfile, like Patch_D2, are reached that way).
//! Game folders are only read; `--out` inside one is refused.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use d2_assets::{GamePath, Profile};

const DEFAULT_EXTS: &str = "ds1,dt1,txt,bin,dat,pl2,tbl,cof,d2";
const DEFAULT_VANILLA: &str = "C:\\Program Files (x86)\\Diablo II";
const DEFAULT_MXL: &str = "C:\\games\\median-xl";

struct Args {
    profile: String,
    out: PathBuf,
    exts: BTreeSet<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut profile = None;
    let mut out = None;
    let mut exts = DEFAULT_EXTS.to_owned();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--profile" => profile = Some(value("--profile")?),
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--ext" => exts = value("--ext")?,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let profile = profile.ok_or("missing --profile vanilla|mxl")?;
    if profile != "vanilla" && profile != "mxl" {
        return Err(format!(
            "unknown profile {profile:?} (expected vanilla or mxl)"
        ));
    }
    Ok(Args {
        profile,
        out: out.ok_or("missing --out <dir>")?,
        exts: exts
            .split(',')
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect(),
    })
}

/// Canonicalise the longest existing ancestor of `path` and re-append the rest,
/// so a not-yet-created `--out` can still be compared with game folders.
fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut existing = absolute.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(ToOwned::to_owned),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    for part in rest.into_iter().rev() {
        out.push(part);
    }
    out
}

fn lower_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn is_inside(path: &Path, dir: &Path) -> bool {
    let (p, d) = (
        lower_components(&resolve(path)),
        lower_components(&resolve(dir)),
    );
    p.starts_with(&d)
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let env_dir = |var: &str, default: &str| {
        PathBuf::from(std::env::var_os(var).unwrap_or_else(|| default.into()))
    };
    let vanilla = env_dir("CAPY_VANILLA_DIR", DEFAULT_VANILLA);
    let mxl = env_dir("CAPY_MXL_DIR", DEFAULT_MXL);
    for game in [&vanilla, &mxl] {
        if is_inside(&args.out, game) {
            return Err(format!(
                "refusing to write inside the game directory {} (--out {})",
                game.display(),
                args.out.display()
            ));
        }
    }
    let install = if args.profile == "vanilla" {
        &vanilla
    } else {
        &mxl
    };
    let profile = if args.profile == "vanilla" {
        Profile::vanilla(install)
    } else {
        Profile::median_xl(install)
    };
    // A saves folder that does not exist: the saves layer stays empty.
    let no_saves = std::env::temp_dir().join(format!("capybara-no-saves-{}", std::process::id()));
    let overlay = profile.open(&no_saves).map_err(|e| e.to_string())?;
    println!(
        "profile {} ({}): {} layers from {}",
        profile.id,
        profile.name,
        overlay.layers().len(),
        install.display()
    );

    // Names: every layer's listfile, plus .bin siblings of excel .txt names.
    let mut names: BTreeSet<GamePath> = BTreeSet::new();
    for layer in overlay.layers() {
        names.extend(layer.list().map_err(|e| e.to_string())?);
    }
    let siblings: Vec<GamePath> = names
        .iter()
        .filter(|n| n.as_str().starts_with("data\\global\\excel\\") && n.extension() == Some("txt"))
        .map(|n| GamePath::new(&format!("{}bin", &n.as_str()[..n.as_str().len() - 3])))
        .collect();
    names.extend(siblings);
    println!("{} candidate names", names.len());

    let mut per_ext: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_layer: BTreeMap<String, usize> = BTreeMap::new();
    let mut bytes_total = 0u64;
    let mut failures = 0usize;
    for name in &names {
        let Some(ext) = name.extension() else {
            continue;
        };
        if !args.exts.contains(ext) {
            continue;
        }
        match overlay.read(name) {
            Ok(Some((layer, data))) => {
                let dest = name.to_native(&args.out);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                std::fs::write(&dest, &data).map_err(|e| format!("{}: {e}", dest.display()))?;
                bytes_total += data.len() as u64;
                *per_ext.entry(ext.to_owned()).or_default() += 1;
                let layer_name = overlay
                    .layers()
                    .get(layer.0)
                    .map_or("?", |l| l.name())
                    .to_owned();
                *per_layer.entry(layer_name).or_default() += 1;
            }
            Ok(None) => {}
            Err(e) => {
                failures += 1;
                eprintln!("failed: {name}: {e}");
            }
        }
    }

    println!("extracted to {}", args.out.display());
    println!("per extension:");
    for (ext, n) in &per_ext {
        println!("  .{ext:<6} {n}");
    }
    println!("per winning layer (priority order):");
    for layer in overlay.layers() {
        println!(
            "  {:<32} {}",
            layer.name(),
            per_layer.get(layer.name()).copied().unwrap_or(0)
        );
    }
    println!(
        "total: {} files, {bytes_total} bytes, {failures} failures",
        per_ext.values().sum::<usize>()
    );
    if failures > 0 {
        return Err(format!("{failures} files failed to read"));
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!("usage: extract --profile vanilla|mxl --out <dir> [--ext {DEFAULT_EXTS}]");
            ExitCode::FAILURE
        }
    }
}
