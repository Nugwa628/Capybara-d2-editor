//! "Save / Discard / Cancel" before a dirty map goes away: the decisions as
//! pure functions of which documents are dirty.

/// What is being closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTarget {
    /// One document tab (by document id).
    Doc(u64),
    /// The whole application.
    Quit,
}

/// A document as the prompt logic sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocFacts {
    pub id: u64,
    pub name: String,
    pub dirty: bool,
}

/// The question put to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosePrompt {
    pub target: CloseTarget,
    /// Dirty documents that would lose their changes.
    pub dirty: Vec<DocFacts>,
}

impl ClosePrompt {
    pub fn title(&self) -> String {
        match (&self.target, self.dirty.as_slice()) {
            (CloseTarget::Doc(_), [one]) => format!("Save changes to {}?", one.name),
            _ => format!("Save changes to {} maps?", self.dirty.len()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosePlan {
    /// Nothing to lose: go ahead.
    CloseNow,
    Ask(ClosePrompt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

/// What to do after the user answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseResolution {
    /// Leave everything as it is.
    Stay,
    /// Save these documents first (in order); close `target` only if every
    /// save succeeded.
    SaveThenClose { save: Vec<u64>, target: CloseTarget },
    /// Close without saving.
    Close { target: CloseTarget },
}

/// Closing one document tab.
pub fn plan_close(docs: &[DocFacts], id: u64) -> ClosePlan {
    match docs.iter().find(|d| d.id == id) {
        Some(d) if d.dirty => ClosePlan::Ask(ClosePrompt {
            target: CloseTarget::Doc(id),
            dirty: vec![d.clone()],
        }),
        _ => ClosePlan::CloseNow,
    }
}

/// Quitting the application.
pub fn plan_quit(docs: &[DocFacts]) -> ClosePlan {
    let dirty: Vec<DocFacts> = docs.iter().filter(|d| d.dirty).cloned().collect();
    if dirty.is_empty() {
        ClosePlan::CloseNow
    } else {
        ClosePlan::Ask(ClosePrompt {
            target: CloseTarget::Quit,
            dirty,
        })
    }
}

pub fn resolve(prompt: &ClosePrompt, choice: CloseChoice) -> CloseResolution {
    match choice {
        CloseChoice::Cancel => CloseResolution::Stay,
        CloseChoice::Discard => CloseResolution::Close {
            target: prompt.target,
        },
        CloseChoice::Save => CloseResolution::SaveThenClose {
            save: prompt.dirty.iter().map(|d| d.id).collect(),
            target: prompt.target,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs() -> Vec<DocFacts> {
        vec![
            DocFacts {
                id: 1,
                name: "a.ds1".into(),
                dirty: false,
            },
            DocFacts {
                id: 2,
                name: "b.ds1".into(),
                dirty: true,
            },
            DocFacts {
                id: 3,
                name: "c.ds1".into(),
                dirty: true,
            },
        ]
    }

    #[test]
    fn a_clean_map_closes_without_asking() {
        assert_eq!(plan_close(&docs(), 1), ClosePlan::CloseNow);
        assert_eq!(plan_close(&docs(), 99), ClosePlan::CloseNow, "unknown id");
    }

    #[test]
    fn a_dirty_map_asks_with_its_name() {
        let ClosePlan::Ask(prompt) = plan_close(&docs(), 2) else {
            unreachable!("a dirty map must ask");
        };
        assert_eq!(prompt.target, CloseTarget::Doc(2));
        assert_eq!(prompt.title(), "Save changes to b.ds1?");
        assert_eq!(prompt.dirty.len(), 1);
    }

    #[test]
    fn quitting_lists_only_the_dirty_maps() {
        let ClosePlan::Ask(prompt) = plan_quit(&docs()) else {
            unreachable!("two dirty maps must ask");
        };
        assert_eq!(prompt.target, CloseTarget::Quit);
        assert_eq!(
            prompt.dirty.iter().map(|d| d.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(prompt.title(), "Save changes to 2 maps?");
        let clean: Vec<DocFacts> = docs().into_iter().filter(|d| !d.dirty).collect();
        assert_eq!(plan_quit(&clean), ClosePlan::CloseNow);
        assert_eq!(plan_quit(&[]), ClosePlan::CloseNow);
    }

    #[test]
    fn the_three_answers() {
        let ClosePlan::Ask(prompt) = plan_quit(&docs()) else {
            unreachable!("must ask");
        };
        assert_eq!(resolve(&prompt, CloseChoice::Cancel), CloseResolution::Stay);
        assert_eq!(
            resolve(&prompt, CloseChoice::Discard),
            CloseResolution::Close {
                target: CloseTarget::Quit
            }
        );
        assert_eq!(
            resolve(&prompt, CloseChoice::Save),
            CloseResolution::SaveThenClose {
                save: vec![2, 3],
                target: CloseTarget::Quit
            }
        );
    }
}
