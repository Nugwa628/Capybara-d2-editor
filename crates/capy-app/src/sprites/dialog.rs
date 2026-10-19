//! Native file dialogs (`rfd`) on a worker thread, so the window keeps
//! drawing while one is open. One dialog at a time; the answer comes back
//! through [`Dialogs::poll`] together with what it was for.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

/// What the picked path(s) will be used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A DC6/DCC/COF anywhere on disk, opened read-only in the Viewer.
    OpenSprite,
    /// Images to import into a new sprite.
    ImportImages,
    /// A folder of frames (+ sprite.json) to import.
    ImportFolder,
    ExportFrames,
    ExportSheet,
    ExportGif,
    ExportPng,
    ExportPaletteGpl,
    ExportPaletteAct,
    ExportPalettePal,
    /// The sprite file itself (DC6/DCC) into a folder of the user's choice.
    ExportSprite,
    BatchInput,
    BatchOutput,
    /// A DT1/DC6/DCC to open in "Convert to palette".
    ConvertOpenFile,
    /// "Save As..." of the converted file.
    ConvertSaveAs,
    /// Files for the palette batch.
    ConvertBatchFiles,
    /// A folder (tree) for the palette batch.
    ConvertBatchFolder,
    ConvertBatchOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    OpenFile,
    OpenFiles,
    Folder,
    Save,
}

impl Purpose {
    fn kind(self) -> Kind {
        match self {
            Purpose::OpenSprite | Purpose::ConvertOpenFile => Kind::OpenFile,
            Purpose::ImportImages | Purpose::ConvertBatchFiles => Kind::OpenFiles,
            Purpose::ConvertBatchFolder | Purpose::ConvertBatchOutput => Kind::Folder,
            Purpose::ConvertSaveAs => Kind::Save,
            Purpose::ImportFolder
            | Purpose::ExportFrames
            | Purpose::BatchInput
            | Purpose::BatchOutput => Kind::Folder,
            Purpose::ExportSheet
            | Purpose::ExportGif
            | Purpose::ExportPng
            | Purpose::ExportPaletteGpl
            | Purpose::ExportPaletteAct
            | Purpose::ExportPalettePal
            | Purpose::ExportSprite => Kind::Save,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Purpose::OpenSprite => "Open a DC6, DCC or COF file",
            Purpose::ImportImages => "Images to import",
            Purpose::ImportFolder => "Folder of frames to import",
            Purpose::ExportFrames => "Folder for the frames and sprite.json",
            Purpose::ExportSheet => "Save the sprite sheet",
            Purpose::ExportGif => "Save the animated GIF",
            Purpose::ExportPng => "Save the frame as an indexed PNG",
            Purpose::ExportPaletteGpl => "Save the palette (GIMP .gpl)",
            Purpose::ExportPaletteAct => "Save the palette (Adobe .act)",
            Purpose::ExportPalettePal => "Save the palette (JASC .pal)",
            Purpose::ExportSprite => "Export the sprite file",
            Purpose::BatchInput => "Batch input folder",
            Purpose::BatchOutput => "Batch output folder",
            Purpose::ConvertOpenFile => "Open a DT1, DC6 or DCC to convert",
            Purpose::ConvertSaveAs => "Save the converted file as",
            Purpose::ConvertBatchFiles => "Files to convert",
            Purpose::ConvertBatchFolder => "Folder to convert (with sub-folders)",
            Purpose::ConvertBatchOutput => "Output folder for the converted files",
        }
    }

    /// (filter name, extensions)
    fn filter(self) -> Option<(&'static str, &'static [&'static str])> {
        match self {
            Purpose::OpenSprite => Some(("Sprites and tiles", &["dc6", "dcc", "cof", "dt1"])),
            Purpose::ConvertOpenFile | Purpose::ConvertBatchFiles | Purpose::ConvertSaveAs => {
                Some(("Tiles and sprites", &["dt1", "dc6", "dcc"]))
            }
            Purpose::ImportImages => Some(("Images", &["png", "bmp", "gif", "pcx"])),
            Purpose::ExportSheet | Purpose::ExportPng => Some(("PNG", &["png"])),
            Purpose::ExportGif => Some(("GIF", &["gif"])),
            Purpose::ExportPaletteGpl => Some(("GIMP palette", &["gpl"])),
            Purpose::ExportPaletteAct => Some(("Adobe colour table", &["act"])),
            Purpose::ExportPalettePal => Some(("JASC palette", &["pal"])),
            Purpose::ExportSprite => Some(("Sprite", &["dc6", "dcc"])),
            _ => None,
        }
    }
}

pub struct Answer {
    pub purpose: Purpose,
    pub paths: Vec<PathBuf>,
}

pub struct Dialogs {
    open: Option<Purpose>,
    tx: Sender<Answer>,
    rx: Mutex<Receiver<Answer>>,
}

impl Default for Dialogs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Dialogs {
            open: None,
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens a dialog unless one is already up. `file_name` prefills save
    /// dialogs.
    pub fn open(&mut self, purpose: Purpose, file_name: Option<String>) {
        if self.open.is_some() {
            return;
        }
        self.open = Some(purpose);
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("file-dialog".to_owned())
            .spawn(move || {
                let mut d = rfd::FileDialog::new().set_title(purpose.title());
                if let Some((name, exts)) = purpose.filter() {
                    d = d.add_filter(name, exts);
                }
                if let Some(name) = file_name {
                    d = d.set_file_name(name);
                }
                let paths: Vec<PathBuf> = match purpose.kind() {
                    Kind::OpenFile => d.pick_file().into_iter().collect(),
                    Kind::OpenFiles => d.pick_files().unwrap_or_default(),
                    Kind::Folder => d.pick_folder().into_iter().collect(),
                    Kind::Save => d.save_file().into_iter().collect(),
                };
                let _ = tx.send(Answer { purpose, paths });
            });
        if spawned.is_err() {
            self.open = None;
        }
    }

    /// The answer of the open dialog once the user closed it (empty paths =
    /// cancelled).
    pub fn poll(&mut self) -> Option<Answer> {
        let answer = self.rx.lock().ok()?.try_recv().ok()?;
        self.open = None;
        Some(answer)
    }
}
