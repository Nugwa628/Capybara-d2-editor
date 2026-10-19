//! The live-search state machine: typing restarts a short timer, the query
//! is sent when the timer runs out, and only the answer to the newest
//! request is shown. Pure (time is passed in, in seconds), so it is tested
//! without a thread or a window.
//!
//! ```text
//!            edit                 due (poll)             complete(latest)
//!   Idle ──────────▶ Waiting ──────────────▶ InFlight ──────────────────▶ Shown
//!                      ▲  │ edit (restart)       │ edit                     │ edit
//!                      └──┘                      ▼                          ▼
//!                                             Waiting                    Waiting
//! ```

/// How long typing has to pause before a search runs.
pub const DEBOUNCE_SECS: f64 = 0.12;

#[derive(Clone, Debug, PartialEq)]
pub struct LiveQuery {
    delay: f64,
    /// The newest wanted key and when it becomes due.
    wanted: Option<(String, f64)>,
    /// The request on its way: id and key.
    in_flight: Option<(u64, String)>,
    /// The key whose answer is on screen.
    shown: Option<String>,
    next_id: u64,
}

impl Default for LiveQuery {
    fn default() -> Self {
        LiveQuery::new(DEBOUNCE_SECS)
    }
}

impl LiveQuery {
    pub fn new(delay: f64) -> LiveQuery {
        LiveQuery {
            delay,
            wanted: None,
            in_flight: None,
            shown: None,
            next_id: 1,
        }
    }

    /// The key the user wants now (the newest of wanted, in flight, shown).
    fn target(&self) -> Option<&str> {
        self.wanted
            .as_ref()
            .map(|(k, _)| k.as_str())
            .or(self.in_flight.as_ref().map(|(_, k)| k.as_str()))
            .or(self.shown.as_deref())
    }

    /// The query (with everything that changes its answer) is now `key`.
    /// Restarts the timer when it changed; same key is a no-op.
    pub fn edit(&mut self, key: &str, now: f64) {
        if self.target() == Some(key) {
            return;
        }
        self.wanted = Some((key.to_owned(), now + self.delay));
    }

    /// Run the wanted query now (Enter, a scope click, a script).
    pub fn flush(&mut self, now: f64) {
        if let Some((_, due)) = self.wanted.as_mut() {
            *due = now.min(*due);
        }
    }

    /// Forget what is shown so the same key is searched again (the index
    /// was rebuilt, an alias changed).
    pub fn invalidate(&mut self, now: f64) {
        if let Some(key) = self.target().map(str::to_owned) {
            self.shown = None;
            self.in_flight = None;
            self.wanted = Some((key, now));
        }
    }

    /// A request to send now: its id and key. The previous request in
    /// flight is superseded (its answer will be ignored).
    pub fn poll(&mut self, now: f64) -> Option<(u64, String)> {
        let due = self.wanted.as_ref().is_some_and(|(_, at)| now >= *at);
        if !due {
            return None;
        }
        let (key, _) = self.wanted.take()?;
        let id = self.next_id;
        self.next_id += 1;
        self.in_flight = Some((id, key.clone()));
        Some((id, key))
    }

    /// An answer arrived. True when it is the newest request's: show it.
    pub fn complete(&mut self, id: u64) -> bool {
        match &self.in_flight {
            Some((latest, _)) if *latest == id => {
                let (_, key) = self.in_flight.take().unwrap_or_default();
                self.shown = Some(key);
                true
            }
            _ => false,
        }
    }

    /// Nothing waiting or in flight, and the answer for `key` is on screen.
    pub fn settled_on(&self, key: &str) -> bool {
        self.wanted.is_none() && self.in_flight.is_none() && self.shown.as_deref() == Some(key)
    }

    pub fn busy(&self) -> bool {
        self.wanted.is_some() || self.in_flight.is_some()
    }

    /// Back to idle (the profile closed).
    pub fn reset(&mut self) {
        self.wanted = None;
        self.in_flight = None;
        self.shown = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_waits_for_a_pause_and_sends_once() {
        let mut q = LiveQuery::new(0.12);
        q.edit("a", 0.0);
        q.edit("ab", 0.05);
        q.edit("abc", 0.10);
        assert_eq!(q.poll(0.15), None, "the timer restarted at 0.10");
        assert!(q.busy());
        let (id, key) = q.poll(0.23).expect("due");
        assert_eq!(key, "abc");
        assert_eq!(q.poll(0.30), None, "sent once");
        assert!(!q.settled_on("abc"));
        assert!(q.complete(id));
        assert!(q.settled_on("abc"));
        assert!(!q.busy());
        // The same key again sends nothing.
        q.edit("abc", 1.0);
        assert_eq!(q.poll(2.0), None);
    }

    #[test]
    fn only_the_newest_answer_is_shown() {
        let mut q = LiveQuery::new(0.1);
        q.edit("a", 0.0);
        let (first, _) = q.poll(0.2).expect("first");
        q.edit("ab", 0.25);
        let (second, key) = q.poll(0.4).expect("second");
        assert_eq!(key, "ab");
        assert!(!q.complete(first), "superseded");
        assert!(!q.settled_on("a"));
        assert!(q.complete(second));
        assert!(q.settled_on("ab"));
        assert!(!q.complete(second), "answered once");
    }

    #[test]
    fn editing_while_in_flight_waits_again() {
        let mut q = LiveQuery::new(0.1);
        q.edit("a", 0.0);
        let (id, _) = q.poll(0.1).expect("sent");
        q.edit("ab", 0.15);
        assert!(q.complete(id), "the older answer may show meanwhile");
        assert!(!q.settled_on("ab"));
        assert_eq!(q.poll(0.2), None, "still debouncing");
        assert_eq!(q.poll(0.25).map(|(_, k)| k), Some("ab".to_owned()));
    }

    #[test]
    fn flush_skips_the_wait_and_invalidate_searches_again() {
        let mut q = LiveQuery::new(10.0);
        q.edit("x", 0.0);
        assert_eq!(q.poll(1.0), None);
        q.flush(1.0);
        let (id, _) = q.poll(1.0).expect("flushed");
        assert!(q.complete(id));
        q.invalidate(5.0);
        assert!(!q.settled_on("x"));
        let (id, key) = q.poll(5.0).expect("again");
        assert_eq!(key, "x");
        assert!(q.complete(id));
        assert!(q.settled_on("x"));
        q.reset();
        assert!(!q.settled_on("x") && !q.busy());
        // Going back to a shown key while another waits still searches it.
        q.edit("y", 6.0);
        let (id, _) = q.poll(17.0).expect("y");
        assert!(q.complete(id));
        q.edit("z", 18.0);
        q.edit("y", 18.1);
        assert_eq!(q.poll(30.0).map(|(_, k)| k), Some("y".to_owned()));
    }
}
