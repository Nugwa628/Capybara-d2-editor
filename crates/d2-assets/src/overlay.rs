//! Priority stack of asset sources.

use crate::error::Result;
use crate::path::GamePath;
use crate::source::{AssetSource, SourceFingerprint};

/// Index of a layer in an [`Overlay`]; 0 is the highest priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub usize);

/// Layers searched top (index 0) to bottom; the first layer holding a file wins.
pub struct Overlay {
    layers: Vec<Box<dyn AssetSource>>,
}

impl Overlay {
    pub fn new(layers: Vec<Box<dyn AssetSource>>) -> Overlay {
        Overlay { layers }
    }

    pub fn layers(&self) -> &[Box<dyn AssetSource>] {
        &self.layers
    }

    /// The highest-priority layer that has `path`.
    pub fn resolve(&self, path: &GamePath) -> Option<LayerId> {
        self.layers
            .iter()
            .position(|l| l.contains(path))
            .map(LayerId)
    }

    /// Every layer that has `path`, top first.
    pub fn resolve_all(&self, path: &GamePath) -> Vec<LayerId> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains(path))
            .map(|(i, _)| LayerId(i))
            .collect()
    }

    /// Contents from the winning layer, with that layer's id.
    pub fn read(&self, path: &GamePath) -> Result<Option<(LayerId, Vec<u8>)>> {
        for (i, layer) in self.layers.iter().enumerate() {
            if let Some(bytes) = layer.read(path)? {
                return Ok(Some((LayerId(i), bytes)));
            }
        }
        Ok(None)
    }

    pub fn read_bytes(&self, path: &GamePath) -> Result<Option<Vec<u8>>> {
        Ok(self.read(path)?.map(|(_, bytes)| bytes))
    }

    /// Contents from one specific layer (`None` for an unknown layer or a
    /// missing file).
    pub fn read_from(&self, layer: LayerId, path: &GamePath) -> Result<Option<Vec<u8>>> {
        match self.layers.get(layer.0) {
            Some(l) => l.read(path),
            None => Ok(None),
        }
    }

    pub fn fingerprints(&self) -> Result<Vec<SourceFingerprint>> {
        self.layers.iter().map(|l| l.fingerprint()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FolderSource;

    fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Box<dyn AssetSource>) {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, text) in files {
            let p = GamePath::new(name).to_native(dir.path());
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(p, text).expect("write");
        }
        let src = FolderSource::new(dir.path(), "t");
        (dir, Box::new(src))
    }

    #[test]
    fn priority_and_resolution() {
        let (_a, top) = folder(&[("data\\x.txt", "top"), ("data\\only_top.txt", "t")]);
        let (_b, bottom) = folder(&[("data\\x.txt", "bottom"), ("data\\only_bottom.txt", "b")]);
        let overlay = Overlay::new(vec![top, bottom]);
        let x = GamePath::new("data\\x.txt");
        assert_eq!(overlay.resolve(&x), Some(LayerId(0)));
        assert_eq!(overlay.resolve_all(&x), vec![LayerId(0), LayerId(1)]);
        assert_eq!(
            overlay.read(&x).expect("read"),
            Some((LayerId(0), b"top".to_vec()))
        );
        assert_eq!(
            overlay.read_from(LayerId(1), &x).expect("from"),
            Some(b"bottom".to_vec())
        );
        let ob = GamePath::new("data\\only_bottom.txt");
        assert_eq!(overlay.resolve(&ob), Some(LayerId(1)));
        assert_eq!(overlay.read_bytes(&ob).expect("bytes"), Some(b"b".to_vec()));
        let none = GamePath::new("data\\none.txt");
        assert_eq!(overlay.resolve(&none), None);
        assert!(overlay.resolve_all(&none).is_empty());
        assert_eq!(overlay.read(&none).expect("none"), None);
        assert_eq!(overlay.read_from(LayerId(9), &x).expect("oob"), None);
        assert_eq!(overlay.fingerprints().expect("fp").len(), 2);
    }
}
