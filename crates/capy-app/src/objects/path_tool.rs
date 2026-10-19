//! The NPC "Path" sub-tool of the Objects tool, as a plain state machine:
//! pointer events in, path commands out. The caller turns the commands into
//! `capy-edit` calls (`add_path_point`, `move_path_point`, ...).
//!
//! * A press on a node selects it and starts a drag; releasing somewhere
//!   else moves the node there (one undo step).
//! * A press on empty ground adds a point there: after the selected node
//!   when one is selected, else at the end of the path. The new point is
//!   selected, so clicking on keeps extending the route.
//! * Delete removes the selected node.

use d2_formats::ds1::PathPoint;

/// Actions a path point can carry. DS1 v15+ stores 1..; the meaning of the
/// values the game uses is not documented (`docs/specs/ds1.md` only says
/// that files below v15 read action 1).
pub const ACTIONS: [(i32, &str); 5] = [
    (1, "1 (default)"),
    (2, "2"),
    (3, "3"),
    (4, "4"),
    (5, "5"),
];

/// The label of an action for combos.
pub fn action_label(action: i32) -> String {
    ACTIONS
        .iter()
        .find(|(a, _)| *a == action)
        .map_or_else(|| action.to_string(), |(_, l)| (*l).to_owned())
}

/// A node being dragged (sub-tile coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathDrag {
    pub point: usize,
    pub start: (i32, i32),
    pub current: (i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathTool {
    /// The selected node of the selected object's path.
    pub selected: Option<usize>,
    pub drag: Option<PathDrag>,
    /// Action given to new points (1..=5).
    pub action: i32,
    /// The object the selection belongs to (a new object resets it).
    pub object: Option<usize>,
}

impl Default for PathTool {
    fn default() -> Self {
        PathTool {
            selected: None,
            drag: None,
            action: 1,
            object: None,
        }
    }
}

/// What the tool asks the document to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathCommand {
    /// `add_path_point(object, point, at)`.
    Add { point: PathPoint, at: Option<usize> },
    /// `move_path_point(object, index, x, y)`.
    Move { index: usize, x: i32, y: i32 },
    /// `delete_path_points(object, [index])`.
    Delete { index: usize },
}

impl PathTool {
    /// Forget the selection when the edited object changes.
    pub fn follow_object(&mut self, object: Option<usize>) {
        if self.object != object {
            self.object = object;
            self.selected = None;
            self.drag = None;
        }
    }

    /// Keep the selection valid for a path of `len` points.
    pub fn clamp(&mut self, len: usize) {
        if self.selected.is_some_and(|s| s >= len) {
            self.selected = len.checked_sub(1);
        }
        if self.drag.is_some_and(|d| d.point >= len) {
            self.drag = None;
        }
    }

    /// Primary press at sub-tile `at`, on node `hit` (if any). `len` is the
    /// path's current point count.
    pub fn press(&mut self, hit: Option<usize>, at: (i32, i32), len: usize) -> Option<PathCommand> {
        self.clamp(len);
        match hit {
            Some(point) if point < len => {
                self.selected = Some(point);
                self.drag = Some(PathDrag {
                    point,
                    start: at,
                    current: at,
                });
                None
            }
            _ => {
                let insert = self.selected.map(|s| s + 1).filter(|&i| i < len);
                self.selected = Some(insert.unwrap_or(len));
                Some(PathCommand::Add {
                    point: PathPoint {
                        x: at.0,
                        y: at.1,
                        action: self.action,
                    },
                    at: insert,
                })
            }
        }
    }

    pub fn drag_to(&mut self, at: (i32, i32)) {
        if let Some(d) = &mut self.drag {
            d.current = at;
        }
    }

    /// Release: a drag that went somewhere moves its node.
    pub fn release(&mut self) -> Option<PathCommand> {
        let d = self.drag.take()?;
        (d.current != d.start).then_some(PathCommand::Move {
            index: d.point,
            x: d.current.0,
            y: d.current.1,
        })
    }

    /// Delete key: remove the selected node; the previous one is selected.
    pub fn delete(&mut self, len: usize) -> Option<PathCommand> {
        self.clamp(len);
        let index = self.selected?;
        self.drag = None;
        self.selected = index.checked_sub(1).or((len > 1).then_some(0));
        Some(PathCommand::Delete { index })
    }

    /// Where node `index` is drawn while dragged, else `None`.
    pub fn dragged(&self, index: usize) -> Option<(i32, i32)> {
        self.drag
            .filter(|d| d.point == index)
            .map(|d| d.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(x: i32, y: i32, action: i32, at: Option<usize>) -> Option<PathCommand> {
        Some(PathCommand::Add {
            point: PathPoint { x, y, action },
            at,
        })
    }

    #[test]
    fn clicks_on_empty_ground_append_and_select_the_new_point() {
        let mut t = PathTool::default();
        assert_eq!(t.press(None, (10, 12), 0), add(10, 12, 1, None));
        assert_eq!(t.selected, Some(0));
        assert_eq!(t.release(), None, "no drag after an add");
        assert_eq!(t.press(None, (14, 12), 1), add(14, 12, 1, None));
        assert_eq!(t.selected, Some(1));
        t.action = 3;
        assert_eq!(t.press(None, (14, 16), 2), add(14, 16, 3, None));
        assert_eq!(t.selected, Some(2));
    }

    #[test]
    fn a_click_after_a_selected_middle_node_inserts_after_it() {
        let mut t = PathTool::default();
        // Select node 0 of a 3-point path, then click on the ground.
        assert_eq!(t.press(Some(0), (1, 1), 3), None);
        assert_eq!(t.release(), None, "a click without moving moves nothing");
        assert_eq!(t.selected, Some(0));
        assert_eq!(t.press(None, (5, 5), 3), add(5, 5, 1, Some(1)));
        assert_eq!(t.selected, Some(1));
        // With the last node selected the point is appended.
        t.selected = Some(3);
        assert_eq!(t.press(None, (6, 6), 4), add(6, 6, 1, None));
        assert_eq!(t.selected, Some(4));
    }

    #[test]
    fn dragging_a_node_moves_it_on_release() {
        let mut t = PathTool::default();
        assert_eq!(t.press(Some(1), (4, 4), 3), None);
        t.drag_to((5, 4));
        assert_eq!(t.dragged(1), Some((5, 4)));
        assert_eq!(t.dragged(0), None);
        t.drag_to((9, 7));
        assert_eq!(
            t.release(),
            Some(PathCommand::Move {
                index: 1,
                x: 9,
                y: 7
            })
        );
        assert!(t.drag.is_none());
        assert_eq!(t.selected, Some(1));
    }

    #[test]
    fn delete_removes_the_selection_and_selects_the_previous_node() {
        let mut t = PathTool::default();
        assert_eq!(t.delete(3), None, "nothing selected");
        t.selected = Some(2);
        assert_eq!(t.delete(3), Some(PathCommand::Delete { index: 2 }));
        assert_eq!(t.selected, Some(1));
        t.selected = Some(0);
        assert_eq!(t.delete(2), Some(PathCommand::Delete { index: 0 }));
        assert_eq!(t.selected, Some(0), "the next node moved into place");
        assert_eq!(t.delete(1), Some(PathCommand::Delete { index: 0 }));
        assert_eq!(t.selected, None, "the path is gone");
    }

    #[test]
    fn a_stale_selection_is_clamped_and_a_new_object_resets() {
        let mut t = PathTool::default();
        t.follow_object(Some(4));
        t.selected = Some(7);
        t.clamp(3);
        assert_eq!(t.selected, Some(2));
        t.follow_object(Some(5));
        assert_eq!(t.selected, None);
        assert_eq!(action_label(1), "1 (default)");
        assert_eq!(action_label(9), "9");
    }
}
