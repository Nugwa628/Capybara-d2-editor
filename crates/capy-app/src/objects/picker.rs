//! The "Add object" picker: a searchable list of what
//! `ObjectCatalog::placeable` offers for the map's act.

use d2_assets::objects::{ObjectClass, ObjectInfo};

/// The picker's state (the list itself is cached in `ObjectsState`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickerState {
    pub query: String,
    /// `(type, id)` of the entry a click on the map places.
    pub chosen: Option<(i32, i32)>,
}

/// `"npc"`, `"object"` or `"type N"`: the class word searches match.
fn class_word(info: &ObjectInfo) -> &'static str {
    match info.class {
        ObjectClass::Npc => "npc monster",
        ObjectClass::Object => "object",
        ObjectClass::Other(_) => "other",
    }
}

/// The text a query is matched against, lower case.
fn haystack(info: &ObjectInfo) -> String {
    format!(
        "{} {} {} type{} id{} {}:{} {}",
        info.name,
        info.token.as_deref().unwrap_or(""),
        info.mode,
        info.ds1_kind,
        info.ds1_id,
        info.ds1_kind,
        info.ds1_id,
        class_word(info)
    )
    .to_ascii_lowercase()
}

/// Indices of `items` matching `query`: every whitespace-separated word must
/// occur in the name, token, mode, `typeN`, `idN`, `type:id` or the class
/// word ("npc", "object"). A bare number also matches the id exactly, and
/// exact id hits come first; otherwise the catalog order is kept.
pub fn filter(items: &[ObjectInfo], query: &str) -> Vec<usize> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    let number: Option<i32> = match words.as_slice() {
        [one] => one.parse().ok(),
        _ => None,
    };
    let mut hits: Vec<(bool, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, info)| {
            let exact = number.is_some_and(|n| info.ds1_id == n);
            let hay = haystack(info);
            let all = words.iter().all(|w| hay.contains(w.as_str()));
            (exact || all).then_some((!exact, i))
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, i)| i).collect()
}

/// One row of the list: `"Torch 2 (type 2, id 37)"`.
pub fn row_label(info: &ObjectInfo) -> String {
    format!(
        "{}  (type {}, id {})",
        info.name, info.ds1_kind, info.ds1_id
    )
}

#[cfg(test)]
pub(crate) fn info(kind: i32, id: i32, name: &str, token: Option<&str>) -> ObjectInfo {
    ObjectInfo {
        class: match kind {
            1 => ObjectClass::Npc,
            2 => ObjectClass::Object,
            k => ObjectClass::Other(k),
        },
        ds1_kind: kind,
        ds1_id: id,
        row: None,
        name: name.to_owned(),
        sprite_base: None,
        token: token.map(str::to_owned),
        mode: "NU".to_owned(),
        weapon_class: "HTH".to_owned(),
        order_flag: 0,
        parts: Vec::new(),
        skins: Vec::new(),
        direction: None,
        size: (0, 0),
        offset: (0, 0),
        draw_effect: None,
        frame_count: None,
        frame_delta: None,
        colormap: None,
        source: "obj.txt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<ObjectInfo> {
        vec![
            info(2, 37, "Torch 1 Tiki", Some("TO")),
            info(1, 12, "Akara", Some("PS")),
            info(2, 12, "Stash", Some("B6")),
            info(1, 120, "Fallen", Some("FA")),
            info(2, 3, "Barrel", Some("B1")),
        ]
    }

    #[test]
    fn empty_query_lists_everything_in_catalog_order() {
        assert_eq!(filter(&items(), ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(filter(&items(), "   "), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn words_match_name_token_class_and_type_case_insensitively() {
        let it = items();
        assert_eq!(filter(&it, "torch"), vec![0]);
        assert_eq!(filter(&it, "TIKI torch"), vec![0]);
        assert_eq!(filter(&it, "npc"), vec![1, 3]);
        assert_eq!(filter(&it, "type2"), vec![0, 2, 4]);
        assert_eq!(filter(&it, "2:12"), vec![2]);
        assert_eq!(filter(&it, "fa"), vec![3], "token FA");
        assert!(filter(&it, "torch akara").is_empty(), "all words must match");
    }

    #[test]
    fn a_number_puts_exact_ids_first() {
        let it = items();
        // id 12 exactly (two of them), then id 120 by substring.
        assert_eq!(filter(&it, "12"), vec![1, 2, 3]);
        assert_eq!(filter(&it, "3"), vec![4, 0], "id 3, then id 37");
        assert_eq!(row_label(&it[0]), "Torch 1 Tiki  (type 2, id 37)");
    }
}
