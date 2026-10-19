//! Batch "Convert to palette": several files or a folder tree (DT1, DC6,
//! DCC) with one source, target and setting set. Overrides are per source
//! index, so they carry across files; each file gets its own plan from its
//! own histogram. Output goes to the profile's saves (CHANGED.txt and a
//! snapshot per file) or to a folder the user picked. Runs on a worker
//! thread with progress and cancel; nothing is written before Start.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use capy_paint::remap::{RemapSettings, plan};
use d2_assets::{GamePath, Overlay};
use d2_formats::palette::Palette;

use crate::paths::Paths;
use crate::sprites::convert::Progress;
use d2_formats::remap::{SpriteKind, index_histogram, remap_indices};

/// Folder walks stop this deep (the robustness audit's limit).
const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchItem {
    /// Read through the profile.
    Game(GamePath),
    /// A loose file; `rel` is its path below the picked folder (or its name).
    File { path: PathBuf, rel: PathBuf },
}

impl BatchItem {
    pub fn name(&self) -> String {
        match self {
            BatchItem::Game(p) => p.as_str().to_owned(),
            BatchItem::File { path, .. } => path.display().to_string(),
        }
    }

    pub fn kind(&self) -> Option<SpriteKind> {
        SpriteKind::from_path(&self.name())
    }

    /// The game path a saves output writes: the item's own, or for a loose
    /// file the part of its path from a `data` folder on.
    pub fn game_path(&self) -> Option<GamePath> {
        match self {
            BatchItem::Game(p) => Some(p.clone()),
            BatchItem::File { path, .. } => {
                let parts: Vec<String> = path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                let at = parts.iter().rposition(|p| p.eq_ignore_ascii_case("data"))?;
                Some(GamePath::new(&parts[at..].join("\\")))
            }
        }
    }

    /// Where a folder output writes, below the output folder.
    pub fn relative(&self) -> PathBuf {
        match self {
            BatchItem::Game(p) => p.as_str().split('\\').collect(),
            BatchItem::File { rel, .. } => rel.clone(),
        }
    }
}

/// The convertible files under `dir` (sorted), each with its path below it.
pub fn collect_folder(dir: &Path, recursive: bool) -> Vec<BatchItem> {
    fn walk(root: &Path, dir: &Path, depth: usize, recursive: bool, out: &mut Vec<BatchItem>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    walk(root, &p, depth + 1, recursive, out);
                }
            } else if SpriteKind::from_path(&p.to_string_lossy()).is_some() {
                let rel = p.strip_prefix(root).map_or_else(
                    |_| p.file_name().map(PathBuf::from).unwrap_or_default(),
                    Path::to_path_buf,
                );
                out.push(BatchItem::File { path: p, rel });
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, 0, recursive, &mut out);
    out
}

/// Loose files picked one by one (written by name to a folder output).
pub fn files_items(files: &[PathBuf]) -> Vec<BatchItem> {
    files
        .iter()
        .filter(|p| SpriteKind::from_path(&p.to_string_lossy()).is_some())
        .map(|p| BatchItem::File {
            path: p.clone(),
            rel: p.file_name().map(PathBuf::from).unwrap_or_default(),
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BatchOutput {
    /// `capybara\saves\<profile>\<game path>` + CHANGED.txt + snapshot.
    Saves {
        profile: String,
    },
    Folder(PathBuf),
}

/// Everything the worker needs.
pub struct BatchJob {
    pub items: Vec<BatchItem>,
    pub src: Palette,
    pub dst: Palette,
    pub settings: RemapSettings,
    pub output: BatchOutput,
    pub overlay: Option<Arc<Overlay>>,
    pub paths: Paths,
    /// Game install folders (never written).
    pub installs: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileDone {
    /// The written file (or game path).
    pub written: String,
    /// The game path, for a saves output.
    pub game: Option<GamePath>,
    pub changed: usize,
    pub mean: f32,
    pub worst: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileResult {
    pub name: String,
    pub result: Result<FileDone, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchReport {
    pub files: Vec<FileResult>,
    pub cancelled: bool,
    /// Files not reached because of a cancel.
    pub left: usize,
}

impl BatchReport {
    pub fn converted(&self) -> usize {
        self.files.iter().filter(|f| f.result.is_ok()).count()
    }

    pub fn failed(&self) -> usize {
        self.files.len() - self.converted()
    }
}

/// Converts one file (read, histogram, plan, remap, write).
pub fn convert_one(item: &BatchItem, job: &BatchJob) -> Result<FileDone, String> {
    let kind = item.kind().ok_or("not a .dt1, .dc6 or .dcc file")?;
    let bytes = match item {
        BatchItem::Game(p) => job
            .overlay
            .as_ref()
            .ok_or("no profile is open")?
            .read_bytes(p)
            .map_err(|e| e.to_string())?
            .ok_or("not in the profile")?,
        BatchItem::File { path, .. } => std::fs::read(path).map_err(|e| e.to_string())?,
    };
    let used = index_histogram(kind, &bytes).map_err(|e| e.to_string())?;
    let p = plan(&job.src, &job.dst, &used, &job.settings);
    let out = remap_indices(kind, &bytes, &p.table).map_err(|e| e.to_string())?;
    let changed = p.entries.iter().filter(|e| e.target != e.source).count();
    let (written, game) = match &job.output {
        BatchOutput::Saves { profile } => {
            let game = item
                .game_path()
                .ok_or("no game path (pick files under a data\\ folder, or a folder output)")?;
            let report = crate::edit::save::save_game_file(&job.paths, profile, &game, &out)?;
            (report.path.display().to_string(), Some(game))
        }
        BatchOutput::Folder(dir) => {
            let file = dir.join(item.relative());
            super::check_not_install(&file, &job.installs)?;
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            std::fs::write(&file, &out)
                .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
            (file.display().to_string(), None)
        }
    };
    Ok(FileDone {
        written,
        game,
        changed,
        mean: p.mean_error,
        worst: p.worst_error,
    })
}

/// Runs the whole job (on the calling thread).
pub fn run(job: &BatchJob, progress: &Mutex<Progress>, cancel: &AtomicBool) -> BatchReport {
    let total = job.items.len();
    let mut report = BatchReport::default();
    for (i, item) in job.items.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            report.left = total - i;
            break;
        }
        if let Ok(mut g) = progress.lock() {
            *g = Progress {
                done: i,
                total,
                current: item.name(),
            };
        }
        report.files.push(FileResult {
            name: item.name(),
            result: convert_one(item, job),
        });
    }
    if let Ok(mut g) = progress.lock() {
        g.done = report.files.len();
        g.total = total;
    }
    report
}

/// The log lines of a finished batch: one per file with its worst error.
pub fn report_lines(report: &BatchReport) -> Vec<String> {
    let mut out = Vec::with_capacity(report.files.len() + 1);
    for f in &report.files {
        out.push(match &f.result {
            Ok(d) => format!(
                "Convert batch: {} -> {}: {} colour(s) changed, mean {:.2}, worst {:.1}",
                f.name, d.written, d.changed, d.mean, d.worst
            ),
            Err(e) => format!("Convert batch failed: {}: {e}", f.name),
        });
    }
    let worst = report
        .files
        .iter()
        .filter_map(|f| f.result.as_ref().ok().map(|d| (d.worst, f.name.as_str())))
        .max_by(|a, b| a.0.total_cmp(&b.0));
    out.push(format!(
        "Convert batch {}: {} converted, {} failed{}{}",
        if report.cancelled {
            "cancelled"
        } else {
            "done"
        },
        report.converted(),
        report.failed(),
        if report.cancelled {
            format!(", {} not reached", report.left)
        } else {
            String::new()
        },
        worst.map_or_else(String::new, |(w, n)| format!("; worst {w:.1} in {n}"))
    ));
    out
}

pub struct PaletteBatchTask {
    pub progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    rx: Mutex<Receiver<BatchReport>>,
}

impl PaletteBatchTask {
    pub fn spawn(job: BatchJob) -> Result<PaletteBatchTask, String> {
        let progress = Arc::new(Mutex::new(Progress {
            total: job.items.len(),
            ..Progress::default()
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (p, c) = (progress.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("palette-batch".to_owned())
            .spawn(move || {
                let _ = tx.send(run(&job, &p, &c));
            })
            .map_err(|e| format!("cannot start the batch: {e}"))?;
        Ok(PaletteBatchTask {
            progress,
            cancel,
            rx: Mutex::new(rx),
        })
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn poll(&self) -> Option<BatchReport> {
        self.rx.lock().ok()?.try_recv().ok()
    }
}

/// The batch section's state.
#[derive(Default)]
pub struct BatchUi {
    pub items: Vec<BatchItem>,
    /// Output to the profile's saves (else `out_dir`).
    pub to_saves: bool,
    pub out_dir: Option<PathBuf>,
    pub recursive: bool,
    /// "Add game files matching" text.
    pub game_filter: String,
    pub task: Option<PaletteBatchTask>,
    /// How the last batch ended, and whether it was cancelled.
    pub outcome: Option<(bool, String)>,
    /// A script started the batch and waits for it.
    pub script_started: bool,
}

impl BatchUi {
    /// Adds items, skipping ones already listed.
    pub fn add(&mut self, items: Vec<BatchItem>) -> usize {
        let before = self.items.len();
        for i in items {
            if !self.items.contains(&i) {
                self.items.push(i);
            }
        }
        self.items.len() - before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capy_sprite::{Sprite, SpriteFormat};
    use d2_formats::IndexedImage;

    fn dc6(dir: &Path, name: &str, indices: &[u8]) -> PathBuf {
        let mut img = IndexedImage::new(indices.len() as u32, 1, 0, -1);
        img.pixels = indices.to_vec();
        img.alpha = vec![255; indices.len()];
        let bytes = Sprite {
            directions: vec![vec![img]],
        }
        .encode(SpriteFormat::Dc6)
        .expect("encode");
        let p = dir.join(name);
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(&p, bytes).expect("write");
        p
    }

    fn palettes() -> (Palette, Palette) {
        let mut a = crate::sprites::palette::grey_palette();
        let mut b = a.clone();
        // index 230 differs: in `b` it is red, and 40 is near-red there.
        a.colors[230] = [200, 10, 10];
        b.colors[230] = [10, 200, 10];
        b.colors[40] = [198, 12, 12];
        a.colors[40] = [198, 12, 12];
        (a, b)
    }

    #[test]
    fn folder_items_and_game_paths() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("ext");
        dc6(&root.join("data").join("global").join("ui"), "a.dc6", &[5]);
        dc6(&root.join("deep"), "b.DC6", &[5]);
        std::fs::write(root.join("notes.txt"), "x").expect("write");
        let items = collect_folder(&root, true);
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].game_path().map(|g| g.as_str().to_owned()),
            Some("data\\global\\ui\\a.dc6".to_owned())
        );
        assert_eq!(items[1].game_path(), None);
        assert_eq!(items[1].relative(), PathBuf::from("deep").join("b.DC6"));
        assert_eq!(collect_folder(&root, false).len(), 0);
        let mut ui = BatchUi::default();
        assert_eq!(ui.add(items.clone()), 2);
        assert_eq!(ui.add(items), 0, "no duplicates");
        assert_eq!(
            BatchItem::Game(GamePath::new("data\\x\\y.dt1")).relative(),
            ["data", "x", "y.dt1"].iter().collect::<PathBuf>()
        );
    }

    #[test]
    fn a_batch_converts_writes_and_reports() {
        let tmp = tempfile::tempdir().expect("tmp");
        let input = tmp.path().join("in");
        dc6(&input, "one.dc6", &[230, 5, 230]);
        dc6(&input.join("sub"), "two.dc6", &[7, 9]);
        std::fs::write(input.join("bad.dc6"), b"junk").expect("write");
        let (src, dst) = palettes();
        let mut settings = RemapSettings::default();
        settings.overrides.insert(9, 11);
        let paths = Paths::at(tmp.path().join("capybara"));
        let out = tmp.path().join("out");
        let job = BatchJob {
            items: collect_folder(&input, true),
            src,
            dst,
            settings,
            output: BatchOutput::Folder(out.clone()),
            overlay: None,
            paths,
            installs: vec![tmp.path().join("game")],
        };
        let progress = Mutex::new(Progress::default());
        let report = run(&job, &progress, &AtomicBool::new(false));
        assert_eq!((report.converted(), report.failed()), (2, 1), "{report:?}");
        let one = Sprite::decode(
            &std::fs::read(out.join("one.dc6")).expect("one"),
            SpriteFormat::Dc6,
        )
        .expect("decode");
        assert_eq!(
            one.directions[0][0].pixels,
            vec![40, 5, 40],
            "230 -> nearest red"
        );
        let two = Sprite::decode(
            &std::fs::read(out.join("sub").join("two.dc6")).expect("two"),
            SpriteFormat::Dc6,
        )
        .expect("decode");
        assert_eq!(
            two.directions[0][0].pixels,
            vec![7, 11],
            "the override carries"
        );
        let lines = report_lines(&report);
        assert!(
            lines
                .last()
                .is_some_and(|l| l.contains("2 converted, 1 failed")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("worst")));
        // Cancelled before the start: nothing is written.
        let cancel = AtomicBool::new(true);
        let r = run(&job, &progress, &cancel);
        assert!(r.cancelled && r.files.is_empty() && r.left == 3);
        // Never into a game install.
        let job = BatchJob {
            output: BatchOutput::Folder(tmp.path().join("game").join("data")),
            ..job
        };
        let r = run(&job, &progress, &AtomicBool::new(false));
        assert_eq!(r.converted(), 0);
    }
}
