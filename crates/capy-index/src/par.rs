//! A tiny scoped worker pool: run `f` over a slice on a few threads while the
//! calling thread polls for progress and cancellation.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::error::{IndexError, Result};

const MAX_WORKERS: usize = 8;
const POLL: Duration = Duration::from_millis(15);

/// Results come back in input order. `on_tick(done)` runs on the calling
/// thread about every 15 ms (and once more at the end); an `Err` from it, or
/// `cancel` being set, stops the workers and is returned
/// ([`IndexError::Cancelled`] for `cancel`).
pub(crate) fn par_map<T: Sync, R: Send>(
    items: &[T],
    cancel: &AtomicBool,
    f: impl Fn(&T) -> R + Sync,
    mut on_tick: impl FnMut(usize) -> Result<()>,
) -> Result<Vec<R>> {
    let n = items.len();
    if n == 0 {
        return Ok(Vec::new());
    }
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let workers = std::thread::available_parallelism()
        .map_or(2, std::num::NonZeroUsize::get)
        .clamp(1, MAX_WORKERS)
        .min(n);
    let mut outcome: Result<()> = Ok(());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = f(item);
                    if let Ok(mut slot) = slots[i].lock() {
                        *slot = Some(result);
                    }
                    done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        loop {
            let d = done.load(Ordering::Relaxed);
            if let Err(e) = on_tick(d) {
                outcome = Err(e);
                stop.store(true, Ordering::Relaxed);
                break;
            }
            if cancel.load(Ordering::Relaxed) {
                outcome = Err(IndexError::Cancelled);
                stop.store(true, Ordering::Relaxed);
                break;
            }
            if d >= n {
                break;
            }
            std::thread::sleep(POLL);
        }
    });
    outcome?;
    let mut out = Vec::with_capacity(n);
    for slot in slots {
        match slot.into_inner() {
            Ok(Some(r)) => out.push(r),
            _ => return Err(IndexError::Other("a worker thread failed".to_owned())),
        }
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_reports() {
        let items: Vec<u32> = (0..200).collect();
        let cancel = AtomicBool::new(false);
        let mut last = 0;
        let out = par_map(
            &items,
            &cancel,
            |x| x * 2,
            |d| {
                last = d;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(out, items.iter().map(|x| x * 2).collect::<Vec<_>>());
        assert_eq!(last, 200);
        assert!(
            par_map(&[] as &[u32], &cancel, |x| *x, |_| Ok(()))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cancel_and_tick_errors_stop_work() {
        let items: Vec<u32> = (0..10_000).collect();
        let cancel = AtomicBool::new(true);
        let r = par_map(&items, &cancel, |x| *x, |_| Ok(()));
        assert!(matches!(r, Err(IndexError::Cancelled)));
        let cancel = AtomicBool::new(false);
        let r = par_map(
            &items,
            &cancel,
            |x| *x,
            |_| Err(IndexError::Other("stop".into())),
        );
        assert!(matches!(r, Err(IndexError::Other(_))));
    }
}
