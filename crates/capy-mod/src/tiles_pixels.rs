//! Walks the palette-index bytes of encoded DT1 sub-blocks.

use d2_formats::dt1::{Block, BlockFormat};

/// Bytes of a drawn isometric sub-block.
const ISO_LEN: usize = 256;

/// Calls `f` on every pixel literal of the block's encoded data (control
/// bytes are never touched). Isometric blocks that are not 256 bytes draw
/// nothing and are skipped. Index 0 literals are passed too; callers decide.
pub(crate) fn for_each_pixel_mut(block: &mut Block, mut f: impl FnMut(&mut u8)) {
    match block.format {
        BlockFormat::Isometric => {
            if block.data.len() == ISO_LEN {
                block.data.iter_mut().for_each(&mut f);
            }
        }
        BlockFormat::Rle => {
            let data = &mut block.data;
            let end = data.len();
            let mut p = 0;
            while p + 1 < end {
                let _skip = data[p];
                let n = usize::from(data[p + 1]);
                p += 2;
                if _skip == 0 && n == 0 {
                    continue;
                }
                let stop = end.min(p + n);
                data[p..stop].iter_mut().for_each(&mut f);
                p = stop;
            }
        }
    }
}

/// Pixel literal counts per palette index (index 0 literals included).
pub(crate) fn histogram(block: &Block, counts: &mut [u64; 256]) {
    let mut copy = block.clone();
    for_each_pixel_mut(&mut copy, |v| counts[usize::from(*v)] += 1);
}
