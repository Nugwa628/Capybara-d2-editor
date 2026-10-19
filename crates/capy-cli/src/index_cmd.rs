//! `index status` and `index build`.

use std::ffi::OsString;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use capy_index::{BuildPhase, Index, IndexStatus};

use crate::{CmdResult, Ctx, Fail, profiles};

/// Progress lines are printed at most this often (phase changes always print).
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

fn phase_name(phase: BuildPhase) -> &'static str {
    match phase {
        BuildPhase::Listing => "listing",
        BuildPhase::Tables => "tables",
        BuildPhase::Maps => "maps",
        BuildPhase::Tiles => "tiles",
        BuildPhase::Sprites => "sprites",
        BuildPhase::Thumbnails => "thumbnails",
        BuildPhase::Writing => "writing",
    }
}

fn open_index(ctx: &Ctx<'_>) -> Result<Index, Fail> {
    let dir = ctx.cache_dir();
    Index::open(&dir)
        .map_err(|e| Fail::Runtime(format!("opening the index in {}: {e}", dir.display())))
}

pub fn status(ctx: &mut Ctx<'_>, profile_arg: &OsString) -> CmdResult {
    let profile = profiles::load(ctx, profile_arg)?;
    let overlay = profiles::open(ctx, &profile)?;
    let index = open_index(ctx)?;
    let status = index
        .status(&profile.id, &overlay)
        .map_err(|e| Fail::Runtime(format!("reading the index status: {e}")))?;
    match status {
        IndexStatus::Missing => ctx.say("missing"),
        IndexStatus::UpToDate => ctx.say("up to date"),
        IndexStatus::Stale { reason } => ctx.say(format!("stale: {reason}")),
    }
    Ok(())
}

pub fn build(ctx: &mut Ctx<'_>, profile_arg: &OsString) -> CmdResult {
    let profile = profiles::load(ctx, profile_arg)?;
    let overlay = profiles::open(ctx, &profile)?;
    let mut index = open_index(ctx)?;
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let mut last: Option<(BuildPhase, Instant)> = None;
    let report = {
        let out = &mut *ctx.out;
        let mut progress = |p: &capy_index::BuildProgress| {
            let now = Instant::now();
            let due = match last {
                Some((phase, at)) => phase != p.phase || now.duration_since(at) >= PROGRESS_EVERY,
                None => true,
            };
            if !due {
                return;
            }
            last = Some((p.phase, now));
            let _ = writeln!(
                out,
                "[{}] {}/{} {}",
                phase_name(p.phase),
                p.done,
                p.total,
                p.current
            );
        };
        index.build(&profile, &overlay, &mut progress, &cancel)
    }
    .map_err(|e| Fail::Runtime(format!("building the index: {e}")))?;
    for w in report.warnings.iter().take(5) {
        ctx.warn(w);
    }
    if report.warnings.len() > 5 {
        ctx.warn(format!(
            "... and {} more warnings",
            report.warnings.len() - 5
        ));
    }
    ctx.say(format!(
        "built index for {}: {} files, {} assets, {} tiles, {} warnings in {:.1}s",
        profile.id,
        report.files,
        report.assets,
        report.tiles,
        report.warnings.len(),
        started.elapsed().as_secs_f64()
    ));
    Ok(())
}
