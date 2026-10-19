//! The OS clipboard side of copy and paste: a clip is also written as JSON
//! so another running instance can paste it, and pasting reads JSON back.

use capy_edit::Clipboard;

/// Puts `text` on the OS clipboard.
pub fn write_os(text: &str) -> Result<(), String> {
    let mut board = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    board.set_text(text.to_owned()).map_err(|e| e.to_string())
}

/// Text on the OS clipboard, if there is any and it can be read.
pub fn read_os() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Which clip a paste uses: JSON from the OS clipboard when it holds a clip
/// (possibly from another instance), else the one held in the app. The flag
/// says the clip came from the OS clipboard.
pub fn choose(os_text: Option<&str>, app: Option<&Clipboard>) -> Option<(Clipboard, bool)> {
    if let Some(clip) = os_text.and_then(Clipboard::from_json) {
        return Some((clip, true));
    }
    app.map(|c| (c.clone(), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use capy_edit::{CellValue, ClipSource, LayerRef};

    fn clip(tag: u32) -> Clipboard {
        Clipboard {
            width: 2,
            height: 1,
            cells: vec![(
                LayerRef::Wall(1),
                1,
                0,
                CellValue {
                    cell: tag,
                    orientation: 3,
                },
            )],
            objects: vec![d2_formats::ds1::Object {
                kind: 1,
                id: 2,
                x: 3,
                y: 4,
                flags: 0,
            }],
            source: ClipSource {
                game_path: Some("data\\a.ds1".into()),
                act: 1,
                lvl_type: Some(5),
                version: 18,
            },
        }
    }

    #[test]
    fn json_round_trips_through_the_os_text() {
        let original = clip(7);
        let text = original.to_json();
        let (back, from_os) = choose(Some(&text), None).expect("clip");
        assert!(from_os);
        assert_eq!(back, original);
    }

    #[test]
    fn foreign_text_falls_back_to_the_app_clipboard() {
        let mine = clip(1);
        let (got, from_os) = choose(Some("hello world"), Some(&mine)).expect("fallback");
        assert!(!from_os);
        assert_eq!(got, mine);
        let (got, _) = choose(None, Some(&mine)).expect("no os text");
        assert_eq!(got, mine);
        assert!(choose(Some("{}"), None).is_none());
        assert!(choose(None, None).is_none());
    }

    #[test]
    fn another_instances_clip_wins_over_ours() {
        let (ours, theirs) = (clip(1), clip(2));
        let (got, from_os) = choose(Some(&theirs.to_json()), Some(&ours)).expect("clip");
        assert!(from_os);
        assert_eq!(got, theirs);
    }
}
