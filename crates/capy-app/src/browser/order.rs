//! Drag-to-reorder for favorites and favorite groups.
//!
//! `UserDb::move_favorite` and `UserDb::move_group` take the position the
//! item gets *after* it is taken out of the list. A favorite group holds
//! the favorites of every profile, while the browser only shows the open
//! profile's, so a drop "before item B" has to be translated into a
//! position in the full list.

/// Where a dragged item lands: before another item, or after the last one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop<T> {
    Before(T),
    End,
}

/// The new position (after removal) for moving `dragged` to `drop` in
/// `items`. `None` when `dragged` is not in the list or the move changes
/// nothing.
pub fn move_position<T: PartialEq>(items: &[T], dragged: &T, drop: &Drop<T>) -> Option<i64> {
    let from = items.iter().position(|i| i == dragged)?;
    let rest: Vec<&T> = items.iter().filter(|i| *i != dragged).collect();
    let to = match drop {
        Drop::Before(target) if target == dragged => return None,
        Drop::Before(target) => rest.iter().position(|i| *i == target)?,
        Drop::End => rest.len(),
    };
    if to == from {
        return None;
    }
    i64::try_from(to).ok()
}

/// [`move_position`] for a group's `(profile, key)` list when only
/// `profile`'s items are shown: "the end" is right after the last shown item.
pub fn favorite_position(
    items: &[(String, String)],
    profile: &str,
    dragged: &str,
    drop: &Drop<String>,
) -> Option<i64> {
    let item = |key: &str| (profile.to_owned(), key.to_owned());
    let dragged_item = item(dragged);
    match drop {
        Drop::Before(target) => move_position(items, &dragged_item, &Drop::Before(item(target))),
        Drop::End => {
            let from = items.iter().position(|i| *i == dragged_item)?;
            let rest: Vec<&(String, String)> =
                items.iter().filter(|i| **i != dragged_item).collect();
            let to = rest
                .iter()
                .rposition(|(p, _)| p == profile)
                .map_or(rest.len(), |last| last + 1);
            if to == from {
                return None;
            }
            i64::try_from(to).ok()
        }
    }
}

/// The new order of `items` after the move (what the database will hold).
#[cfg(test)]
pub fn apply<T: Clone + PartialEq>(items: &[T], dragged: &T, position: i64) -> Vec<T> {
    let mut out: Vec<T> = items.iter().filter(|i| *i != dragged).cloned().collect();
    let to = usize::try_from(position).unwrap_or(0).min(out.len());
    out.insert(to, dragged.clone());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_move_before_a_target_or_to_the_end() {
        let ids = [10, 20, 30, 40];
        // 40 before 20.
        let p = move_position(&ids, &40, &Drop::Before(20)).expect("moves");
        assert_eq!(apply(&ids, &40, p), vec![10, 40, 20, 30]);
        // 10 before 30.
        let p = move_position(&ids, &10, &Drop::Before(30)).expect("moves");
        assert_eq!(apply(&ids, &10, p), vec![20, 10, 30, 40]);
        // 20 to the end.
        let p = move_position(&ids, &20, &Drop::End).expect("moves");
        assert_eq!(apply(&ids, &20, p), vec![10, 30, 40, 20]);
        // No-ops: onto itself, before its successor, the last to the end.
        assert_eq!(move_position(&ids, &20, &Drop::Before(20)), None);
        assert_eq!(move_position(&ids, &20, &Drop::Before(30)), None);
        assert_eq!(move_position(&ids, &40, &Drop::End), None);
        // Unknown items.
        assert_eq!(move_position(&ids, &99, &Drop::End), None);
        assert_eq!(move_position(&ids, &10, &Drop::Before(99)), None);
    }

    fn fav(p: &str, k: &str) -> (String, String) {
        (p.to_owned(), k.to_owned())
    }

    #[test]
    fn favorites_of_other_profiles_keep_their_places() {
        // Group order: a(mxl) x(vanilla) b(mxl) c(mxl) y(vanilla).
        let items = vec![
            fav("mxl", "a"),
            fav("vanilla", "x"),
            fav("mxl", "b"),
            fav("mxl", "c"),
            fav("vanilla", "y"),
        ];
        // The mxl view shows a b c; drag c before a.
        let p =
            favorite_position(&items, "mxl", "c", &Drop::Before("a".to_owned())).expect("moves");
        let after = apply(&items, &fav("mxl", "c"), p);
        let shown: Vec<&str> = after
            .iter()
            .filter(|(p, _)| p == "mxl")
            .map(|(_, k)| k.as_str())
            .collect();
        assert_eq!(shown, vec!["c", "a", "b"]);
        assert_eq!(after.len(), 5);
        // Drag a to the end of the mxl view: after c, before vanilla's y.
        let p = favorite_position(&items, "mxl", "a", &Drop::End).expect("moves");
        let after = apply(&items, &fav("mxl", "a"), p);
        assert_eq!(
            after,
            vec![
                fav("vanilla", "x"),
                fav("mxl", "b"),
                fav("mxl", "c"),
                fav("mxl", "a"),
                fav("vanilla", "y"),
            ]
        );
        // c is already the last mxl item.
        assert_eq!(favorite_position(&items, "mxl", "c", &Drop::End), None);
        // Only this profile's keys are found.
        assert_eq!(favorite_position(&items, "mxl", "x", &Drop::End), None);
    }
}
