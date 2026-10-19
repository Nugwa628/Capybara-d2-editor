//! Large deterministic payloads with an embedded checksum (shared by the crash test and its helper).

pub const PAYLOAD_LEN: usize = 3 * 1024 * 1024;

/// `PAYLOAD_LEN` pseudo-random bytes derived from `seed`, the last 32 being the blake3 of the rest.
pub fn payload(seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(PAYLOAD_LEN);
    while out.len() < PAYLOAD_LEN - 32 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(PAYLOAD_LEN - 32);
    let sum = blake3::hash(&out);
    out.extend_from_slice(sum.as_bytes());
    out
}

/// True when the embedded checksum matches the content.
#[allow(dead_code)]
pub fn checksum_ok(bytes: &[u8]) -> bool {
    if bytes.len() != PAYLOAD_LEN {
        return false;
    }
    let (body, sum) = bytes.split_at(PAYLOAD_LEN - 32);
    blake3::hash(body).as_bytes() == sum
}
