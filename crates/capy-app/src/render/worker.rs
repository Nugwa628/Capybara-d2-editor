//! The map build thread. One long-lived thread runs every build job in
//! order. While the jobs come from the same overlay and object tables it
//! keeps one `ObjectSprites`, so COF and DCC decodes are cached across
//! edits and the sprite frame `Arc`s (the atlas keys of object layers) stay
//! the same from build to build, which lets an edit reuse the atlas.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use capy_render::{ObjectSprites, TileSet};
use d2_assets::objects::ObjectCatalog;
use d2_assets::{GamePath, Overlay};
use d2_formats::ds1::Ds1;

use super::atlas::AtlasLayout;
use super::job::{self, BuildInput, BuildKey, BuildOutput, ObjectSource};

pub struct JobRequest {
    pub doc: u64,
    pub ticket: u64,
    pub div: u32,
    pub tick: Option<u32>,
    pub key: BuildKey,
    pub profile: String,
    pub overlay: Arc<Overlay>,
    pub tiles: Option<Arc<TileSet>>,
    pub ds1: Arc<Ds1>,
    pub prior: Option<Arc<AtlasLayout>>,
    /// The object tables; `None` draws terrain only.
    pub catalog: Option<Arc<ObjectCatalog>>,
}

/// A finished build, sent back from the worker thread.
pub struct JobResult {
    pub doc: u64,
    pub ticket: u64,
    pub div: u32,
    pub tick: Option<u32>,
    pub key: BuildKey,
    pub profile: String,
    pub tiles: Option<Arc<TileSet>>,
    pub warnings: Vec<String>,
    pub outcome: Result<BuildOutput, String>,
}

impl JobRequest {
    fn fail(self, text: String) -> JobResult {
        JobResult {
            doc: self.doc,
            ticket: self.ticket,
            div: self.div,
            tick: self.tick,
            key: self.key,
            profile: self.profile,
            tiles: None,
            warnings: Vec::new(),
            outcome: Err(text),
        }
    }

    fn same_objects(&self, overlay: &Arc<Overlay>, catalog: &Arc<ObjectCatalog>) -> bool {
        Arc::ptr_eq(&self.overlay, overlay)
            && self
                .catalog
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, catalog))
    }
}

fn run_job(req: JobRequest, objects: Option<&ObjectSource<'_>>) -> JobResult {
    let JobRequest {
        doc,
        ticket,
        div,
        tick,
        key,
        profile,
        overlay,
        tiles,
        ds1,
        prior,
        catalog: _,
    } = req;
    let mut warnings = Vec::new();
    let mut loaded: Option<Arc<TileSet>> = None;
    let outcome = (|| {
        let tiles = match tiles {
            Some(t) => t,
            None => {
                let (t, w) = job::load_tileset(&overlay, &key.dt1_files)?;
                warnings = w;
                t
            }
        };
        loaded = Some(tiles.clone());
        let palette = job::load_palette(&overlay, key.palette_act)?;
        job::build_map(
            BuildInput {
                tiles,
                ds1,
                vis: key.vis.clone(),
                palette,
                palette_key: key.palette_key(),
                prior,
                div,
                tick,
                filter: key.filter.clone(),
            },
            objects,
        )
    })();
    JobResult {
        doc,
        ticket,
        div,
        tick,
        key,
        profile,
        tiles: loaded,
        warnings,
        outcome,
    }
}

/// `run_job` that turns a panic into a failed build.
fn run_guarded(req: JobRequest, objects: Option<&ObjectSource<'_>>) -> JobResult {
    let (doc, ticket, div, tick) = (req.doc, req.ticket, req.div, req.tick);
    let (key, profile) = (req.key.clone(), req.profile.clone());
    crate::crash::catch(|| run_job(req, objects)).unwrap_or_else(|_| JobResult {
        doc,
        ticket,
        div,
        tick,
        key,
        profile,
        tiles: None,
        warnings: Vec::new(),
        outcome: Err("map build panicked".to_owned()),
    })
}

fn worker_loop(rx: Receiver<JobRequest>, tx: Sender<JobResult>) {
    let mut pending: Option<JobRequest> = None;
    loop {
        let req = match pending.take() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(r) => r,
                Err(_) => return,
            },
        };
        let Some(catalog) = req.catalog.clone() else {
            if tx.send(run_guarded(req, None)).is_err() {
                return;
            }
            continue;
        };
        // One sprite cache for as long as the jobs share overlay and tables.
        let overlay = req.overlay.clone();
        let sprites = ObjectSprites::new(&overlay, &catalog);
        let source = ObjectSource {
            sprites: &sprites,
            catalog: &catalog,
        };
        let mut current = req;
        loop {
            if tx.send(run_guarded(current, Some(&source))).is_err() {
                return;
            }
            match rx.recv() {
                Ok(next) if next.same_objects(&overlay, &catalog) => current = next,
                Ok(next) => {
                    pending = Some(next);
                    break;
                }
                Err(_) => return,
            }
        }
    }
}

/// The handle of the build thread.
pub struct Worker {
    tx: Option<Sender<JobRequest>>,
    results: Sender<JobResult>,
}

impl Worker {
    /// Starts the thread; results arrive on `results`.
    pub fn spawn(results: Sender<JobResult>) -> Worker {
        let (tx, rx) = channel();
        let thread_results = results.clone();
        let spawned = std::thread::Builder::new()
            .name("map-build".to_owned())
            .spawn(move || worker_loop(rx, thread_results));
        Worker {
            tx: spawned.ok().map(|_| tx),
            results,
        }
    }

    /// Queues a build. A dead thread answers with a failed build.
    pub fn send(&self, req: JobRequest) {
        let req = match &self.tx {
            Some(tx) => match tx.send(req) {
                Ok(()) => return,
                Err(e) => e.0,
            },
            None => req,
        };
        let _ = self
            .results
            .send(req.fail("the map build thread is not running".to_owned()));
    }
}

/// The DT1 list and profile a tileset cache entry is for.
pub type TilesetKey = (String, Vec<GamePath>);
