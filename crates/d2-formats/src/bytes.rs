//! Bounds-checked little-endian reads shared by the wave-2 parsers.

pub(crate) fn u16_at(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

pub(crate) fn i16_at(bytes: &[u8], off: usize) -> Option<i16> {
    u16_at(bytes, off).map(|v| v as i16)
}

pub(crate) fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([
        *b.first()?,
        *b.get(1)?,
        *b.get(2)?,
        *b.get(3)?,
    ]))
}

pub(crate) fn i32_at(bytes: &[u8], off: usize) -> Option<i32> {
    u32_at(bytes, off).map(|v| v as i32)
}

/// `bytes[off..off + len]`, or `None` when that range leaves the slice.
pub(crate) fn slice_at(bytes: &[u8], off: usize, len: usize) -> Option<&[u8]> {
    bytes.get(off..off.checked_add(len)?)
}
