//! Sub-tile collision flags of DT1 tiles (ds1-studio `collisionFlags.ts`,
//! `docs/collision-flags.md`).
//!
//! A tile has 25 flag bytes, one per sub-tile of its 5x5 footprint, indexed
//! `y * 5 + x` (as `Tile::subtile_flags`). Each byte is eight independent
//! flags, not exclusive types.

use d2_formats::dt1::Dt1;

/// One collision bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionFlag {
    pub bit: u8,
    pub name: &'static str,
    /// Advanced flags are shown, but only preserved or edited deliberately.
    pub advanced: bool,
    pub help: &'static str,
}

/// The eight bits and what the classic engine does with them.
pub const COLLISION_FLAGS: [CollisionFlag; 8] = [
    CollisionFlag {
        bit: 0x01,
        name: "Block walking",
        advanced: false,
        help: "Blocks ordinary walking for players, mercenaries and monsters. Does not by itself prevent flying or teleporting.",
    },
    CollisionFlag {
        bit: 0x02,
        name: "Sight / shooting barrier",
        advanced: false,
        help: "An obstacle for sight and shooting checks that use the visible collision mask.",
    },
    CollisionFlag {
        bit: 0x04,
        name: "Jump / flight barrier",
        advanced: false,
        help: "Used by jump and flight checks. Teleport also depends on the level's Teleport setting.",
    },
    CollisionFlag {
        bit: 0x08,
        name: "Block player walking",
        advanced: false,
        help: "Blocks ordinary player paths; monster and pet path masks do not use this bit.",
    },
    CollisionFlag {
        bit: 0x10,
        name: "Preset / placement",
        advanced: true,
        help: "The classic engine's PRESET bit, used in placement masks. Not a general missile blocker.",
    },
    CollisionFlag {
        bit: 0x20,
        name: "Light / blank",
        advanced: true,
        help: "Block Light in DT1 tools, BLANK in the engine; used by some movement checks.",
    },
    CollisionFlag {
        bit: 0x40,
        name: "Missile mask bit",
        advanced: true,
        help: "Overlaps the dynamic missile occupancy bit; authored meaning not established.",
    },
    CollisionFlag {
        bit: 0x80,
        name: "Player mask bit",
        advanced: true,
        help: "Overlaps the dynamic player occupancy bit; authored meaning not established.",
    },
];

/// Walking, player walking and jump/flight: what "make walkable" clears.
pub const WALKABLE_MASK: u8 = 0x0D;

/// Two-digit upper-case hex of a flag byte.
pub fn collision_hex(value: u8) -> String {
    format!("{value:02X}")
}

/// `"Block walking + Jump / flight barrier"`, or the empty-flags text.
pub fn describe_collision(value: u8) -> String {
    if value == 0 {
        return "No DT1 collision flags".to_string();
    }
    COLLISION_FLAGS
        .iter()
        .filter(|f| value & f.bit != 0)
        .map(|f| f.name)
        .collect::<Vec<_>>()
        .join(" + ")
}

/// How a brush changes a flag byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagOp {
    /// OR the bits in.
    Add(u8),
    /// Clear only these bits.
    Remove(u8),
    /// Replace all eight bits (`Set(0)` clears them all).
    Set(u8),
    /// Clear [`WALKABLE_MASK`], keep the other bits.
    MakeWalkable,
}

impl FlagOp {
    pub fn apply(self, value: u8) -> u8 {
        match self {
            FlagOp::Add(b) => value | b,
            FlagOp::Remove(b) => value & !b,
            FlagOp::Set(b) => b,
            FlagOp::MakeWalkable => value & !WALKABLE_MASK,
        }
    }
}

/// One sub-tile whose byte changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlagChange {
    pub tile: usize,
    /// `y * 5 + x`.
    pub subtile: usize,
    pub before: u8,
    pub after: u8,
}

/// Why a flag edit could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagEditError {
    NoSuchTile(usize),
    BadSubtile(usize),
}

impl std::fmt::Display for FlagEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlagEditError::NoSuchTile(t) => write!(f, "the DT1 has no tile #{t}"),
            FlagEditError::BadSubtile(s) => write!(f, "sub-tile {s} is outside the 5x5 grid"),
        }
    }
}

impl std::error::Error for FlagEditError {}

/// Applies `op` to the listed sub-tiles (`y * 5 + x`) of the listed tiles
/// (file order). All requests are checked first: on error nothing changes.
/// Only bytes that really change are returned and written.
pub fn edit_subtile_flags(
    dt1: &mut Dt1,
    tiles: &[usize],
    subtiles: &[usize],
    op: FlagOp,
) -> Result<Vec<FlagChange>, FlagEditError> {
    if let Some(&t) = tiles.iter().find(|&&t| t >= dt1.tiles.len()) {
        return Err(FlagEditError::NoSuchTile(t));
    }
    if let Some(&s) = subtiles.iter().find(|&&s| s >= 25) {
        return Err(FlagEditError::BadSubtile(s));
    }
    let mut changes = Vec::new();
    for &t in tiles {
        let Some(tile) = dt1.tiles.get_mut(t) else {
            continue;
        };
        for &s in subtiles {
            let Some(byte) = tile.subtile_flags.get_mut(s) else {
                continue;
            };
            let after = op.apply(*byte);
            if after != *byte {
                changes.push(FlagChange {
                    tile: t,
                    subtile: s,
                    before: *byte,
                    after,
                });
                *byte = after;
            }
        }
    }
    Ok(changes)
}

/// Replaces all 25 bytes of one tile (the "pick combination" / paste case).
pub fn set_tile_flags(
    dt1: &mut Dt1,
    tile: usize,
    flags: [u8; 25],
) -> Result<Vec<FlagChange>, FlagEditError> {
    let t = dt1
        .tiles
        .get_mut(tile)
        .ok_or(FlagEditError::NoSuchTile(tile))?;
    let mut changes = Vec::new();
    for (s, (&new, old)) in flags.iter().zip(t.subtile_flags.iter_mut()).enumerate() {
        if new != *old {
            changes.push(FlagChange {
                tile,
                subtile: s,
                before: *old,
                after: new,
            });
            *old = new;
        }
    }
    Ok(changes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn ops() {
        assert_eq!(FlagOp::Add(0x04).apply(0x01), 0x05);
        assert_eq!(FlagOp::Remove(0x01).apply(0x07), 0x06);
        assert_eq!(FlagOp::Set(0).apply(0xFF), 0);
        assert_eq!(FlagOp::MakeWalkable.apply(0xFF), 0xF2);
        assert_eq!(
            describe_collision(0x05),
            "Block walking + Jump / flight barrier"
        );
        assert_eq!(describe_collision(0), "No DT1 collision flags");
        assert_eq!(collision_hex(0x0a), "0A");
    }
}
