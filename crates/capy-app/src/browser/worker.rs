//! The search worker: one thread with its own `Index` and `UserDb`
//! connections (SQLite connections are not `Sync`), answering searches,
//! thumbnails, asset lookups and "used in" lists so the UI thread never
//! waits on the database.
//!
//! Searches jump the queue and an older search of the same channel is
//! dropped unanswered when a newer one is already waiting; the UI shows the
//! newest answer only anyway (see [`super::live::LiveQuery`]).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::time::Instant;

use capy_index::{AssetRow, Hit, Index, Query, UserDb};

use super::scope::merge_hits;

/// Who asked for a search: the Tiles browser or the Ctrl+K palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    Browser,
    Palette,
}

pub enum Request {
    Search {
        channel: Channel,
        id: u64,
        profile: String,
        /// Run each, then merge (see `scope::scoped_queries`).
        queries: Vec<Query>,
        limit: usize,
    },
    Thumb {
        asset_id: i64,
    },
    Lookup {
        profile: String,
        keys: Vec<String>,
    },
    UsedIn {
        asset_id: i64,
    },
}

/// A decoded thumbnail.
pub struct Thumb {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub enum Response {
    Search {
        channel: Channel,
        id: u64,
        result: Result<Vec<Hit>, String>,
        millis: u64,
    },
    Thumb {
        asset_id: i64,
        image: Option<Thumb>,
    },
    Lookup {
        profile: String,
        rows: Vec<(String, Option<AssetRow>)>,
    },
    UsedIn {
        asset_id: i64,
        maps: Result<Vec<(String, u32)>, String>,
    },
}

pub struct Worker {
    tx: Sender<Request>,
    rx: Mutex<Receiver<Response>>,
}

impl Worker {
    /// Starts the thread. The connections open on the first request.
    pub fn spawn(cache_dir: PathBuf, user_db: PathBuf) -> Result<Worker, String> {
        let (tx, requests) = channel::<Request>();
        let (answers, rx) = channel::<Response>();
        std::thread::Builder::new()
            .name("asset-search".to_owned())
            .spawn(move || {
                let mut db = Db {
                    cache_dir,
                    user_db,
                    open: None,
                };
                let _ = crate::crash::catch(|| {
                    serve(&mut db, &requests, &answers);
                });
            })
            .map_err(|e| format!("cannot start the search thread: {e}"))?;
        Ok(Worker {
            tx,
            rx: Mutex::new(rx),
        })
    }

    /// False when the thread is gone.
    pub fn send(&self, request: Request) -> bool {
        self.tx.send(request).is_ok()
    }

    pub fn drain(&self) -> Vec<Response> {
        match self.rx.lock() {
            Ok(rx) => rx.try_iter().collect(),
            Err(_) => Vec::new(),
        }
    }
}

struct Db {
    cache_dir: PathBuf,
    user_db: PathBuf,
    open: Option<(Index, UserDb)>,
}

impl Db {
    fn get(&mut self) -> Result<&(Index, UserDb), String> {
        if self.open.is_none() {
            let index = Index::open(&self.cache_dir).map_err(|e| e.to_string())?;
            let user = UserDb::open(&self.user_db).map_err(|e| e.to_string())?;
            self.open = Some((index, user));
        }
        self.open
            .as_ref()
            .ok_or_else(|| "the index is not open".to_owned())
    }
}

/// Requests waiting, by priority.
#[derive(Default)]
struct Queue {
    searches: Vec<Request>,
    other: VecDeque<Request>,
    thumbs: VecDeque<Request>,
}

impl Queue {
    fn push(&mut self, request: Request) {
        match request {
            Request::Search { channel, .. } => {
                // A newer search of the same channel replaces a waiting one.
                self.searches
                    .retain(|r| !matches!(r, Request::Search { channel: c, .. } if *c == channel));
                self.searches.push(request);
            }
            Request::Thumb { .. } => self.thumbs.push_back(request),
            other => self.other.push_back(other),
        }
    }

    fn pop(&mut self) -> Option<Request> {
        if !self.searches.is_empty() {
            return Some(self.searches.remove(0));
        }
        // Newest thumbnails first: they are the ones on screen now.
        self.other.pop_front().or_else(|| self.thumbs.pop_back())
    }
}

fn serve(db: &mut Db, requests: &Receiver<Request>, answers: &Sender<Response>) {
    let mut queue = Queue::default();
    loop {
        // Take everything that is waiting; block only when idle.
        loop {
            match requests.try_recv() {
                Ok(r) => queue.push(r),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(request) = queue.pop() else {
            match requests.recv() {
                Ok(r) => {
                    queue.push(r);
                    continue;
                }
                Err(_) => return,
            }
        };
        let answer = handle(db, request);
        if answers.send(answer).is_err() {
            return;
        }
    }
}

fn handle(db: &mut Db, request: Request) -> Response {
    match request {
        Request::Search {
            channel,
            id,
            profile,
            queries,
            limit,
        } => {
            let started = Instant::now();
            let result = db.get().and_then(|(index, user)| {
                let mut lists = Vec::with_capacity(queries.len());
                for q in &queries {
                    lists.push(
                        index
                            .search(&profile, q, user, limit)
                            .map_err(|e| e.to_string())?,
                    );
                }
                Ok(merge_hits(lists, limit))
            });
            Response::Search {
                channel,
                id,
                result,
                millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }
        }
        Request::Thumb { asset_id } => {
            let image = db
                .get()
                .ok()
                .and_then(|(index, _)| index.thumbnail(asset_id).ok().flatten())
                .and_then(|png| decode_png(&png));
            Response::Thumb { asset_id, image }
        }
        Request::Lookup { profile, keys } => {
            let rows = match db.get() {
                Ok((index, _)) => keys
                    .into_iter()
                    .map(|k| {
                        let row = index.asset(&profile, &k).ok().flatten();
                        (k, row)
                    })
                    .collect(),
                Err(_) => keys.into_iter().map(|k| (k, None)).collect(),
            };
            Response::Lookup { profile, rows }
        }
        Request::UsedIn { asset_id } => {
            let maps = db
                .get()
                .and_then(|(index, _)| index.used_in(asset_id).map_err(|e| e.to_string()));
            Response::UsedIn { asset_id, maps }
        }
    }
}

/// RGBA pixels of a PNG thumbnail; `None` for anything unreadable.
pub fn decode_png(bytes: &[u8]) -> Option<Thumb> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }
    Some(Thumb {
        width: w as usize,
        height: h as usize,
        rgba: rgba.into_raw(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(channel: Channel, id: u64) -> Request {
        Request::Search {
            channel,
            id,
            profile: "p".into(),
            queries: Vec::new(),
            limit: 1,
        }
    }

    fn id_of(r: &Request) -> String {
        match r {
            Request::Search { channel, id, .. } => format!("{channel:?}{id}"),
            Request::Thumb { asset_id } => format!("t{asset_id}"),
            Request::Lookup { keys, .. } => format!("l{}", keys.len()),
            Request::UsedIn { asset_id } => format!("u{asset_id}"),
        }
    }

    #[test]
    fn searches_jump_the_queue_and_replace_older_ones() {
        let mut q = Queue::default();
        q.push(Request::Thumb { asset_id: 1 });
        q.push(search(Channel::Browser, 1));
        q.push(Request::UsedIn { asset_id: 7 });
        q.push(Request::Thumb { asset_id: 2 });
        q.push(search(Channel::Palette, 2));
        q.push(search(Channel::Browser, 3));
        let order: Vec<String> = std::iter::from_fn(|| q.pop()).map(|r| id_of(&r)).collect();
        assert_eq!(order, vec!["Palette2", "Browser3", "u7", "t2", "t1"]);
    }

    #[test]
    fn png_thumbnails_decode_and_garbage_does_not() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode");
        let t = decode_png(&png).expect("decodes");
        assert_eq!((t.width, t.height, t.rgba.len()), (3, 2, 24));
        assert!(decode_png(b"not a png").is_none());
    }

    /// The real thread over an empty cache: searches answer (with nothing),
    /// thumbnails of unknown assets come back empty.
    #[test]
    fn the_thread_answers_over_an_empty_index() {
        let tmp = tempfile::tempdir().expect("tmp");
        let worker =
            Worker::spawn(tmp.path().join("cache"), tmp.path().join("user.db")).expect("spawn");
        assert!(worker.send(search(Channel::Browser, 5)));
        assert!(worker.send(Request::Thumb { asset_id: 3 }));
        assert!(worker.send(Request::Lookup {
            profile: "p".into(),
            keys: vec!["k".into()],
        }));
        let mut got = Vec::new();
        let started = Instant::now();
        while got.len() < 3 && started.elapsed().as_secs() < 20 {
            got.extend(worker.drain());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got.len(), 3);
        for r in got {
            match r {
                Response::Search { id, result, .. } => {
                    assert_eq!(id, 5);
                    assert!(result.expect("ok").is_empty());
                }
                Response::Thumb { image, .. } => assert!(image.is_none()),
                Response::Lookup { rows, .. } => assert_eq!(rows[0].1, None),
                Response::UsedIn { .. } => unreachable!("not asked"),
            }
        }
    }
}
