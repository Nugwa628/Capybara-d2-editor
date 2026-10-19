//! Sector / single-unit decompression.
//!
//! Failures are reported as `(method, detail)`; the caller wraps them into
//! `AssetError::Decompress` with archive and file names.

use std::io::Read;

pub(crate) const METHOD_HUFFMAN: u8 = 0x01;
pub(crate) const METHOD_ZLIB: u8 = 0x02;
pub(crate) const METHOD_PKWARE: u8 = 0x08;
pub(crate) const METHOD_BZIP2: u8 = 0x10;
pub(crate) const METHOD_SPARSE: u8 = 0x20;
pub(crate) const METHOD_ADPCM_MONO: u8 = 0x40;
pub(crate) const METHOD_ADPCM_STEREO: u8 = 0x80;

pub(crate) type DecompressError = (u8, String);

/// Decompress one stored unit into exactly `expected` bytes.
///
/// * `implode` is the block's IMPLODE flag (0x100): the data is a bare PKWARE
///   DCL stream with no mask byte.
/// * otherwise the first byte is the compression mask (COMPRESS flag, 0x200).
pub(crate) fn decompress(
    data: &[u8],
    expected: usize,
    implode: bool,
) -> Result<Vec<u8>, DecompressError> {
    if implode {
        let out = run(METHOD_PKWARE, data, expected)?;
        if out.len() != expected {
            return Err((
                METHOD_PKWARE,
                format!("expected {expected} bytes, got {}", out.len()),
            ));
        }
        return Ok(out);
    }
    let Some((&mask, body)) = data.split_first() else {
        return Err((0, "empty compressed unit".into()));
    };
    if mask == 0 {
        return Err((0, "compression mask is zero".into()));
    }
    // Decompression order: bzip2, PKWARE, zlib, then the wave codecs.
    const ORDER: [u8; 7] = [
        METHOD_BZIP2,
        METHOD_PKWARE,
        METHOD_ZLIB,
        METHOD_HUFFMAN,
        METHOD_ADPCM_STEREO,
        METHOD_ADPCM_MONO,
        METHOD_SPARSE,
    ];
    let known = ORDER.iter().fold(0u8, |acc, m| acc | m);
    if mask & !known != 0 {
        return Err((
            mask,
            format!("unknown compression bits in mask {mask:#04x}"),
        ));
    }
    let mut current: Option<Vec<u8>> = None;
    for method in ORDER {
        if mask & method == 0 {
            continue;
        }
        let input: &[u8] = current.as_deref().unwrap_or(body);
        let out = run(method, input, expected)?;
        current = Some(out);
    }
    let out = current.unwrap_or_default();
    if out.len() != expected {
        return Err((
            mask,
            format!("expected {expected} bytes, got {}", out.len()),
        ));
    }
    Ok(out)
}

fn run(method: u8, input: &[u8], expected: usize) -> Result<Vec<u8>, DecompressError> {
    // Never produce more than `expected + 1` bytes, so a bomb costs nothing.
    let limit = (expected as u64).saturating_add(1);
    let mut out = Vec::with_capacity(expected.min(1 << 20));
    let result = match method {
        METHOD_ZLIB => flate2::read::ZlibDecoder::new(input)
            .take(limit)
            .read_to_end(&mut out)
            .map(|_| ()),
        METHOD_BZIP2 => bzip2::read::BzDecoder::new(input)
            .take(limit)
            .read_to_end(&mut out)
            .map(|_| ()),
        METHOD_PKWARE => explode::ExplodeReader::new(input)
            .take(limit)
            .read_to_end(&mut out)
            .map(|_| ()),
        METHOD_HUFFMAN => return Err((method, "Huffman decompression is not supported".into())),
        METHOD_ADPCM_MONO | METHOD_ADPCM_STEREO => {
            return Err((method, "ADPCM decompression is not supported".into()));
        }
        METHOD_SPARSE => return Err((method, "sparse decompression is not supported".into())),
        other => return Err((other, "unknown compression method".into())),
    };
    if let Err(e) = result {
        return Err((method, e.to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "AIAIAIAIAIAIA" from the `explode` crate's documentation.
    const IMPLODED: [u8; 8] = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8F, 0x80, 0x7F];

    #[test]
    fn implode_stream() {
        let out = decompress(&IMPLODED, 13, true).expect("explode");
        assert_eq!(out, b"AIAIAIAIAIAIA");
    }

    #[test]
    fn implode_with_mask_byte() {
        let mut data = vec![METHOD_PKWARE];
        data.extend_from_slice(&IMPLODED);
        assert_eq!(
            decompress(&data, 13, false).expect("explode"),
            b"AIAIAIAIAIAIA"
        );
    }

    #[test]
    fn wrong_length_is_an_error() {
        assert!(decompress(&IMPLODED, 12, true).is_err());
        assert!(decompress(&IMPLODED, 14, true).is_err());
    }

    #[test]
    fn truncated_and_mangled_implode_never_panic() {
        for cut in 0..IMPLODED.len() {
            let _ = decompress(&IMPLODED[..cut], 13, true);
        }
        for i in 0..IMPLODED.len() {
            for bit in 0..8 {
                let mut d = IMPLODED;
                d[i] ^= 1 << bit;
                let _ = decompress(&d, 13, true);
            }
        }
    }

    #[test]
    fn unsupported_methods_error() {
        for mask in [METHOD_HUFFMAN, 0x41, 0x81, METHOD_SPARSE, 0x04, 0] {
            let data = [mask, 1, 2, 3, 4];
            assert!(decompress(&data, 4, false).is_err(), "mask {mask:#x}");
        }
        assert!(decompress(&[], 4, false).is_err());
    }
}
