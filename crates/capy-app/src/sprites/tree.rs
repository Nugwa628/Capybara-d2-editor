//! The Sprites list as a folder tree: the game path below `data\`, folders
//! first, natural order, single-child folder chains shown as one row.
//! Pure data; `ui/sprites/browser.rs` draws it.

use std::collections::HashSet;

use d2_assets::GamePath;

/// One tree node. Folders hold child node indices, files an index into
/// `Session::sprites`.
pub enum Node {
    Folder {
        /// `monsters › zm` (a collapsed chain joins its names).
        label: String,
        /// Full path of the deepest folder of the chain: the open/closed key.
        key: String,
        /// Visible files below.
        count: usize,
        children: Vec<usize>,
    },
    File {
        sprite: usize,
        name: String,
    },
}

#[derive(Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

/// Cache key: filter text, type toggles, session.
pub type Key = (String, [bool; 4], u64);

/// The list's tree, its cache key and which folders are open.
#[derive(Default)]
pub struct TreeState {
    pub key: Option<Key>,
    pub tree: Tree,
    pub open: HashSet<String>,
    /// The sprite whose folders were last opened for the viewer.
    pub revealed: Option<GamePath>,
}

/// Order of two game paths: per component, folders before files, natural
/// case-insensitive order within each.
pub fn path_order(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ia = a.split('\\').peekable();
    let mut ib = b.split('\\').peekable();
    loop {
        match (ia.next(), ib.next()) {
            (Some(x), Some(y)) => {
                let (fx, fy) = (ia.peek().is_none(), ib.peek().is_none());
                // `false` (a folder, more components follow) sorts first.
                let o = fx.cmp(&fy).then_with(|| natural(x, y));
                if o.is_ne() {
                    return o;
                }
            }
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
        }
    }
}

/// Case-insensitive natural order: digit runs compare by value
/// (`a2` < `a10`), everything else char by char.
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.next_if(char::is_ascii_digit) {
                        s.push(c);
                    }
                    s
                };
                let (da, db) = (run(&mut a), run(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let o = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| da.len().cmp(&db.len()));
                if o.is_ne() {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if o.is_ne() {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Folder components of `path` below `data\`, and its file name.
pub fn split(path: &str) -> (Vec<&str>, &str) {
    let mut parts: Vec<&str> = path.split('\\').collect();
    let name = parts.pop().unwrap_or("");
    if parts
        .first()
        .is_some_and(|p| p.eq_ignore_ascii_case("data"))
    {
        parts.remove(0);
    }
    (parts, name)
}

struct Raw {
    name: String,
    key: String,
    folders: Vec<Raw>,
    files: Vec<(usize, String)>,
    count: usize,
}

impl Raw {
    fn new(name: &str, key: String) -> Raw {
        Raw {
            name: name.to_owned(),
            key,
            folders: Vec::new(),
            files: Vec::new(),
            count: 0,
        }
    }
}

fn flatten(mut cur: Raw, out: &mut Tree) -> usize {
    // Collapse `a` -> `b` when `a` holds nothing else.
    let mut label = std::mem::take(&mut cur.name);
    while cur.files.is_empty() && cur.folders.len() == 1 {
        let next = cur.folders.pop().expect("one folder");
        label = format!("{label} \u{203a} {}", next.name);
        cur = next;
    }
    let mut children = Vec::new();
    for f in cur.folders {
        children.push(flatten(f, out));
    }
    for (sprite, name) in cur.files {
        out.nodes.push(Node::File { sprite, name });
        children.push(out.nodes.len() - 1);
    }
    out.nodes.push(Node::Folder {
        label,
        key: cur.key,
        count: cur.count,
        children,
    });
    out.nodes.len() - 1
}

/// Builds the tree of `sprites[i]` for the `filtered` indices; `sprites` is
/// in `path_order` (see `sprite_list`) so folders arrive grouped.
pub fn build(sprites: &[GamePath], filtered: &[usize]) -> Tree {
    let mut root = Raw::new("", String::new());
    for &i in filtered {
        let (dirs, name) = split(sprites[i].as_str());
        let mut cur = &mut root;
        cur.count += 1;
        for d in dirs {
            // Input is sorted, so a folder is either new or the last one.
            if !cur.folders.last().is_some_and(|f| f.name == d) {
                let key = if cur.key.is_empty() {
                    d.to_owned()
                } else {
                    format!("{}\\{d}", cur.key)
                };
                cur.folders.push(Raw::new(d, key));
            }
            cur = cur.folders.last_mut().expect("just pushed");
            cur.count += 1;
        }
        cur.files.push((i, name.to_owned()));
    }
    let mut tree = Tree::default();
    let mut roots = Vec::new();
    for f in root.folders {
        roots.push(flatten(f, &mut tree));
    }
    for (sprite, name) in root.files {
        tree.nodes.push(Node::File { sprite, name });
        roots.push(tree.nodes.len() - 1);
    }
    tree.roots = roots;
    tree
}

/// A visible row.
#[derive(Clone, Copy)]
pub struct Row {
    pub node: usize,
    pub depth: usize,
}

/// Rows of the open folders (all of them when `all_open`).
pub fn rows(tree: &Tree, open: &HashSet<String>, all_open: bool) -> Vec<Row> {
    let mut out = Vec::new();
    let mut stack: Vec<Row> = tree
        .roots
        .iter()
        .rev()
        .map(|&node| Row { node, depth: 0 })
        .collect();
    while let Some(r) = stack.pop() {
        out.push(r);
        if let Node::Folder { key, children, .. } = &tree.nodes[r.node]
            && (all_open || open.contains(key))
        {
            stack.extend(children.iter().rev().map(|&node| Row {
                node,
                depth: r.depth + 1,
            }));
        }
    }
    out
}

/// Every folder key of the path (`a`, `a\b`, ...), to open it in the tree.
pub fn ancestors(path: &str) -> Vec<String> {
    let (dirs, _) = split(path);
    let mut out = Vec::new();
    let mut acc = String::new();
    for d in dirs {
        if !acc.is_empty() {
            acc.push('\\');
        }
        acc.push_str(d);
        out.push(acc.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(s: &str) -> GamePath {
        GamePath::new(s)
    }

    #[test]
    fn folders_first_natural_order() {
        let mut v = vec![
            "data\\global\\a.dc6",
            "data\\global\\act10\\x.dc6",
            "data\\global\\act2\\x10.dc6",
            "data\\global\\act2\\x2.dc6",
            "data\\global\\act2\\x.dc6",
            "data\\global\\zed\\x.dc6",
        ];
        v.sort_by(|a, b| path_order(a, b));
        assert_eq!(
            v,
            vec![
                "data\\global\\act2\\x.dc6",
                "data\\global\\act2\\x2.dc6",
                "data\\global\\act2\\x10.dc6",
                "data\\global\\act10\\x.dc6",
                "data\\global\\zed\\x.dc6",
                "data\\global\\a.dc6",
            ]
        );
    }

    fn sample() -> Vec<GamePath> {
        [
            "data\\global\\monsters\\zm\\tr\\a1.dcc",
            "data\\global\\monsters\\zm\\tr\\a2.dcc",
            "data\\global\\monsters\\zm\\tr\\a10.dcc",
            "data\\global\\tiles\\act1\\town\\f.dt1",
            "data\\global\\tiles\\act1\\g.dt1",
            "data\\local\\ui\\x.dc6",
        ]
        .map(gp)
        .to_vec()
    }

    fn label(t: &Tree, n: usize) -> (String, usize) {
        match &t.nodes[n] {
            Node::Folder { label, count, .. } => (label.clone(), *count),
            Node::File { name, .. } => (name.clone(), 0),
        }
    }

    #[test]
    fn builds_counts_and_collapses_chains() {
        let s = sample();
        let all: Vec<usize> = (0..s.len()).collect();
        let t = build(&s, &all);
        let roots: Vec<_> = t.roots.iter().map(|&r| label(&t, r)).collect();
        assert_eq!(
            roots,
            [
                ("global".to_owned(), 5),
                ("local \u{203a} ui".to_owned(), 1)
            ]
        );
        assert_eq!(rows(&t, &HashSet::new(), false).len(), 2);
        let open: HashSet<String> = ["global".to_owned()].into();
        let names: Vec<_> = rows(&t, &open, false)
            .iter()
            .map(|r| label(&t, r.node).0)
            .collect();
        assert_eq!(
            names,
            [
                "global",
                "monsters \u{203a} zm \u{203a} tr",
                "tiles \u{203a} act1",
                "local \u{203a} ui"
            ]
        );
        // Files in natural order inside a folder.
        let files: Vec<_> = rows(&t, &HashSet::new(), true)
            .iter()
            .map(|r| label(&t, r.node).0)
            .filter(|n| n.ends_with(".dcc"))
            .collect();
        assert_eq!(files, ["a1.dcc", "a2.dcc", "a10.dcc"]);
    }

    #[test]
    fn filter_hides_empty_folders_and_counts() {
        let s = sample();
        let t = build(&s, &[3, 4]);
        assert_eq!(t.roots.len(), 1);
        assert_eq!(
            label(&t, t.roots[0]),
            ("global \u{203a} tiles \u{203a} act1".to_owned(), 2)
        );
        assert!(build(&s, &[]).roots.is_empty());
    }

    #[test]
    fn ancestors_list_prefixes() {
        assert_eq!(
            ancestors("data\\global\\tiles\\act1\\g.dt1"),
            ["global", "global\\tiles", "global\\tiles\\act1"]
        );
    }
}
