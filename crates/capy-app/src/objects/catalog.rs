//! The object tables (`ObjectCatalog`) of the open profile, built once per
//! session on a worker thread. The map renderer, the object picker, the
//! properties panel and the Sprite preview all share it.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use d2_assets::Overlay;
use d2_assets::objects::ObjectCatalog;

/// What the build thread sends: the tables and their warnings.
type Built = Result<(ObjectCatalog, Vec<String>), String>;

pub enum CatalogSlot {
    None,
    Building {
        epoch: u64,
        rx: Mutex<Receiver<Built>>,
    },
    Ready {
        epoch: u64,
        catalog: Arc<ObjectCatalog>,
    },
    Failed {
        epoch: u64,
        error: String,
    },
}

/// What [`CatalogSlot::poll`] has to say this frame.
pub enum CatalogPoll {
    /// Still building (or not started: no profile).
    Pending,
    Ready(Arc<ObjectCatalog>),
    Failed(String),
}

/// What changed in the last poll, for the log.
pub enum CatalogNews {
    Ready { warnings: Vec<String> },
    Failed(String),
}

impl CatalogSlot {
    fn epoch(&self) -> Option<u64> {
        match self {
            CatalogSlot::Building { epoch, .. }
            | CatalogSlot::Ready { epoch, .. }
            | CatalogSlot::Failed { epoch, .. } => Some(*epoch),
            CatalogSlot::None => None,
        }
    }

    /// Starts the build for `epoch` when the slot holds another epoch.
    pub fn ensure(&mut self, epoch: u64, overlay: &Arc<Overlay>, version: d2_assets::GameVersion) {
        if self.epoch() == Some(epoch) {
            return;
        }
        let (tx, rx) = channel();
        let overlay = overlay.clone();
        let spawned = std::thread::Builder::new()
            .name("object-catalog".to_owned())
            .spawn(move || {
                let r = crate::crash::catch(|| {
                    ObjectCatalog::build(&overlay, version).map_err(|e| e.to_string())
                })
                .unwrap_or_else(|_| Err("building the object tables panicked".to_owned()));
                let _ = tx.send(r);
            });
        *self = match spawned {
            Ok(_) => CatalogSlot::Building {
                epoch,
                rx: Mutex::new(rx),
            },
            Err(e) => CatalogSlot::Failed {
                epoch,
                error: e.to_string(),
            },
        };
    }

    /// Collects a finished build. Returns news the first time it is ready or
    /// failed.
    pub fn collect(&mut self) -> Option<CatalogNews> {
        let CatalogSlot::Building { epoch, rx } = self else {
            return None;
        };
        let epoch = *epoch;
        let got = rx.get_mut().ok().and_then(|r| r.try_recv().ok());
        match got {
            Some(Ok((catalog, warnings))) => {
                *self = CatalogSlot::Ready {
                    epoch,
                    catalog: Arc::new(catalog),
                };
                Some(CatalogNews::Ready { warnings })
            }
            Some(Err(error)) => {
                *self = CatalogSlot::Failed {
                    epoch,
                    error: error.clone(),
                };
                Some(CatalogNews::Failed(error))
            }
            None => None,
        }
    }

    /// The catalog of `epoch`, if it is built.
    pub fn poll(&self, epoch: u64) -> CatalogPoll {
        match self {
            CatalogSlot::Ready { epoch: e, catalog } if *e == epoch => {
                CatalogPoll::Ready(catalog.clone())
            }
            CatalogSlot::Failed { epoch: e, error } if *e == epoch => {
                CatalogPoll::Failed(error.clone())
            }
            _ => CatalogPoll::Pending,
        }
    }

    pub fn ready(&self, epoch: u64) -> Option<Arc<ObjectCatalog>> {
        match self.poll(epoch) {
            CatalogPoll::Ready(c) => Some(c),
            _ => None,
        }
    }
}
