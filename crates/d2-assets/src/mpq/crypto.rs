//! MPQ hashing and the Storm block cipher.

use std::sync::OnceLock;

pub(crate) const HASH_TABLE_OFFSET: u32 = 0;
pub(crate) const HASH_NAME_A: u32 = 1;
pub(crate) const HASH_NAME_B: u32 = 2;
pub(crate) const HASH_FILE_KEY: u32 = 3;

const TABLE_LEN: usize = 0x500;

/// The 1280-entry table all MPQ hashing and encryption derive from.
fn crypt_table() -> &'static [u32; TABLE_LEN] {
    static TABLE: OnceLock<[u32; TABLE_LEN]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; TABLE_LEN];
        let mut seed: u32 = 0x0010_0001;
        for index1 in 0..0x100usize {
            let mut index2 = index1;
            for _ in 0..5 {
                seed = (seed.wrapping_mul(125).wrapping_add(3)) % 0x002A_AAAB;
                let temp1 = (seed & 0xFFFF) << 16;
                seed = (seed.wrapping_mul(125).wrapping_add(3)) % 0x002A_AAAB;
                let temp2 = seed & 0xFFFF;
                table[index2] = temp1 | temp2;
                index2 += 0x100;
            }
        }
        table
    })
}

/// Look up `table[index]`; `index` is always in range by construction, the
/// `get` only exists so no code path can panic.
fn table_at(index: usize) -> u32 {
    crypt_table().get(index).copied().unwrap_or(0)
}

/// MPQ name hash. `hash_type`: 0 = table offset, 1 = name A, 2 = name B,
/// 3 = file/table key. Case-insensitive (ASCII), `/` is treated as `\`.
pub fn hash_string(name: &str, hash_type: u32) -> u32 {
    let mut seed1: u32 = 0x7FED_7FED;
    let mut seed2: u32 = 0xEEEE_EEEE;
    let base = ((hash_type & 0xF) as usize) << 8;
    for byte in name.bytes() {
        let ch = match byte {
            b'/' => b'\\',
            b => b.to_ascii_uppercase(),
        };
        seed1 = table_at(base + ch as usize) ^ seed1.wrapping_add(seed2);
        seed2 = (ch as u32)
            .wrapping_add(seed1)
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
    seed1
}

/// Decrypt `data` in place. Only whole little-endian `u32` words are
/// processed, a trailing partial word is left untouched (as Storm does).
pub(crate) fn decrypt_bytes(data: &mut [u8], key: u32) {
    let mut key = key;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for chunk in data.as_chunks_mut::<4>().0 {
        seed2 = seed2.wrapping_add(table_at(0x400 + (key & 0xFF) as usize));
        let plain = u32::from_le_bytes(*chunk) ^ key.wrapping_add(seed2);
        *chunk = plain.to_le_bytes();
        key = ((!key) << 0x15).wrapping_add(0x1111_1111) | (key >> 0x0B);
        seed2 = plain
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
}

/// Inverse of [`decrypt_bytes`]. Only used by tests (the editor never writes
/// archives), but kept next to its inverse.
#[cfg(test)]
pub(crate) fn encrypt_bytes(data: &mut [u8], key: u32) {
    let mut key = key;
    let mut seed2: u32 = 0xEEEE_EEEE;
    for chunk in data.as_chunks_mut::<4>().0 {
        seed2 = seed2.wrapping_add(table_at(0x400 + (key & 0xFF) as usize));
        let plain = u32::from_le_bytes(*chunk);
        *chunk = (plain ^ key.wrapping_add(seed2)).to_le_bytes();
        key = ((!key) << 0x15).wrapping_add(0x1111_1111) | (key >> 0x0B);
        seed2 = plain
            .wrapping_add(seed2)
            .wrapping_add(seed2 << 5)
            .wrapping_add(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hashes() {
        assert_eq!(hash_string("(hash table)", HASH_FILE_KEY), 0xC3AF_3770);
        assert_eq!(hash_string("(block table)", HASH_FILE_KEY), 0xEC83_B3A3);
    }

    #[test]
    fn hash_is_case_and_slash_insensitive() {
        for t in 0..4 {
            assert_eq!(
                hash_string("Data/Global\\Excel/Levels.BIN", t),
                hash_string("DATA\\GLOBAL\\EXCEL\\LEVELS.BIN", t)
            );
        }
        assert_ne!(hash_string("a", 1), hash_string("b", 1));
    }

    #[test]
    fn crypt_table_spot_values() {
        // First entry of the well-known table.
        assert_eq!(table_at(0), 0x55C6_36E2);
    }

    #[test]
    fn cipher_round_trip() {
        let original: Vec<u8> = (0..1031u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut data = original.clone();
        encrypt_bytes(&mut data, 0xDEAD_BEEF);
        assert_ne!(data, original);
        // The trailing 3 bytes are not a whole word and stay as they were.
        assert_eq!(data[1028..], original[1028..]);
        decrypt_bytes(&mut data, 0xDEAD_BEEF);
        assert_eq!(data, original);
    }
}
