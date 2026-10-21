//! The player skill sequences: the hardcoded frame orders the game plays for
//! skills with `anim = SQ` (Leap, Whirlwind, Arctic Blast, ...), indexed by
//! the `seqnum` column of `skills.txt`.
//!
//! Such a skill has no COF of its own. The game looks up the sequence for
//! `seqnum` and the unit's weapon class, then plays its steps in order; each
//! step names a mode (`a1`, `a2`, `sc`, `kk`, ...), a frame of that mode's COF
//! for the same token and weapon class, and the event that frame raises.
//!
//! Source: the D2MOO decompilation of Diablo II 1.10 (D2Common.dll),
//! <https://github.com/ThePhrozenKeep/D2MOO>, commit
//! `98a3cc281a6a96708e881718934b8e4c99bbe9e2`,
//! `source/D2Common/src/DataTbls/SequenceTbls.cpp` (`gPlayerSequence*` step
//! arrays, `gPlayerWeapons*` per-weapon-class records and the 24-entry
//! `gPlayerWeaponsSequenceTable` that `seqnum` indexes; slot 0 is NULL). The
//! tables were converted mechanically and nothing was added by hand. The
//! names are D2MOO's, not the game's. Sequences 1..=23 are the player ones;
//! larger numbers in `skills.txt` index `monseq.txt` (monsters) instead.
//!
//! The steps depend on the weapon class (a record has 14 slots, in the order
//! of [`CLASS_CODES`]) and a slot may be empty: the game has no sequence for
//! that class. 1.10 is the only version with decompiled data; the tables are
//! code in D2Common.dll, so later versions are assumed (not verified) to
//! carry the same ones.
//!
//! Two arrays (Fend with a spear, Projection) end with an
//! all-zero entry: mode 0 (`dt`), frame 0. It is padding in the original;
//! [`PlayerSequence::steps`] drops it.

use crate::txt::TxtTable;

/// Weapon class codes in the slot order of a sequence record. The game finds
/// a slot through D2MOO's `gWeaponIndexToClassMap`, whose order this is. It is
/// not the order of D2MOO's `D2WeaponClasses` enum, which the names of D2MOO's
/// step arrays (`Jab_BOW`, `Strafe_2HS`, `Sacrifice_1HT`, ...) follow, so those
/// names are misleading; the arrays here are named for the class they serve.
/// Evidence for this order: Strafe (a bow skill) fills only slots 5 and 6
/// (bow, crossbow); Whirlwind and Charge leave 5 and 6 empty (no bow or
/// crossbow); Jab, Impale and Fend fill slots 1 and 2 (javelin, spear);
/// Sacrifice fills 3 and 4 (one- and two-hand swing); Charge leaves every
/// dual-wield slot (8 and up) empty (paladins cannot dual-wield).
pub const CLASS_CODES: [&str; 14] = [
    "hth", "1ht", "2ht", "1hs", "2hs", "bow", "xbw", "stf", "1js", "1jt", "1ss", "1st", "ht1",
    "ht2",
];

/// What a step raises when it is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqEvent {
    None,
    /// The hit frame of a melee attack.
    MeleeAttack,
    MissileAttack,
    PlaySound,
    TriggerSkill,
}

/// One entry of a sequence: play `frame` of the COF for `mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqStep {
    /// Two-letter mode code (`a1`, `a2`, `sc`, `kk`, `s1`, ...).
    pub mode: &'static str,
    pub frame: u8,
    pub event: SeqEvent,
}

impl SeqStep {
    /// The all-zero entry some arrays end with.
    fn is_padding(&self) -> bool {
        self.mode == "dt" && self.frame == 0 && self.event == SeqEvent::None
    }
}

/// One sequence: its number and the steps per weapon class.
#[derive(Clone, Copy, Debug)]
pub struct PlayerSequence {
    pub seqnum: u8,
    /// D2MOO's name, spaced ("Leap Attack").
    pub name: &'static str,
    /// By weapon class slot ([`CLASS_CODES`]); `None` when the game has none.
    pub by_class: [Option<&'static [SeqStep]>; 14],
}

impl PlayerSequence {
    /// The steps for a weapon class code (`"1hs"`, any case), without the
    /// zero padding some arrays end with.
    pub fn steps(&self, class: &str) -> Option<&'static [SeqStep]> {
        let steps = self.by_class[class_slot(class)?]?;
        let end = steps
            .iter()
            .rposition(|s| !s.is_padding())
            .map_or(0, |i| i + 1);
        Some(&steps[..end])
    }
}

/// The slot of a weapon class code (`"hth"`, `"1hs"`, ...), ignoring case.
pub fn class_slot(class: &str) -> Option<usize> {
    CLASS_CODES
        .iter()
        .position(|c| c.eq_ignore_ascii_case(class))
}

/// The player sequence for a `skills.txt` `seqnum` (1..=23).
pub fn sequence(seqnum: u32) -> Option<&'static PlayerSequence> {
    let i = usize::try_from(seqnum.checked_sub(1)?).ok()?;
    PLAYER_SEQUENCES.get(i)
}

/// The `skills.txt` `charclass` code of a character token (`ba` -> `bar`).
pub fn class_of_token(token: &str) -> Option<&'static str> {
    Some(match token.to_ascii_lowercase().as_str() {
        "am" => "ama",
        "so" => "sor",
        "ne" => "nec",
        "pa" => "pal",
        "ba" => "bar",
        "dz" => "dru",
        "ai" => "ass",
        _ => return None,
    })
}

/// A player skill that plays a sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqSkill {
    pub name: String,
    pub seqnum: u32,
    /// The `charclass` code (`bar`, `ama`, ...), lower case.
    pub class: String,
}

/// The player skills of a `skills.txt` that carry a sequence number, by
/// sequence number then name. Rows with no `charclass` (monster skills) are
/// skipped; their numbers belong to `monseq.txt`.
pub fn skills_using(skills: &TxtTable) -> Vec<SeqSkill> {
    let mut out: Vec<SeqSkill> = skills
        .rows()
        .filter(|r| r.get("charclass").is_some_and(|c| !c.trim().is_empty()))
        .filter_map(|r| {
            let seqnum = u32::try_from(r.get_i64("seqnum")?).ok()?;
            let name = r.get("skill")?.trim();
            let class = r.get("charclass")?.trim().to_ascii_lowercase();
            (sequence(seqnum).is_some() && !name.is_empty()).then(|| SeqSkill {
                name: name.to_owned(),
                seqnum,
                class,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.seqnum, &a.name).cmp(&(b.seqnum, &b.name)));
    out
}

const N: SeqEvent = SeqEvent::None;
const MELEE: SeqEvent = SeqEvent::MeleeAttack;
const SOUND: SeqEvent = SeqEvent::PlaySound;

const fn st(mode: &'static str, frame: u8, event: SeqEvent) -> SeqStep {
    SeqStep { mode, frame, event }
}

static HAND_TO_HAND1: [SeqStep; 13] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a1", 7, N),
    st("a1", 8, MELEE),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
];

static JAB_1HT: [SeqStep; 18] = [
    st("a1", 5, N),
    st("a1", 6, SOUND),
    st("a1", 8, N),
    st("a1", 9, MELEE),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 13, N),
    st("a2", 6, SOUND),
    st("a2", 8, N),
    st("a2", 9, MELEE),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 13, N),
    st("a2", 6, SOUND),
    st("a2", 8, N),
    st("a2", 9, MELEE),
    st("a2", 10, N),
    st("a2", 13, N),
];

static JAB_2HT: [SeqStep; 21] = [
    st("a1", 2, N),
    st("a1", 7, SOUND),
    st("a1", 9, N),
    st("a1", 10, MELEE),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 15, N),
    st("a2", 4, N),
    st("a2", 6, SOUND),
    st("a2", 9, N),
    st("a2", 10, MELEE),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 15, N),
    st("a2", 4, N),
    st("a2", 6, SOUND),
    st("a2", 9, N),
    st("a2", 10, MELEE),
    st("a2", 11, N),
    st("a2", 13, N),
    st("a2", 15, N),
];

static HAND_TO_HAND2: [SeqStep; 14] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a1", 7, MELEE),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
];

static SACRIFICE_1HS: [SeqStep; 7] = [
    st("a2", 0, N),
    st("a2", 2, N),
    st("a2", 4, N),
    st("a2", 7, MELEE),
    st("a2", 8, N),
    st("a2", 10, N),
    st("a2", 13, N),
];

static SACRIFICE_2HS: [SeqStep; 8] = [
    st("a1", 0, N),
    st("a1", 2, N),
    st("a1", 4, N),
    st("a1", 6, N),
    st("a1", 8, MELEE),
    st("a1", 10, N),
    st("a1", 13, N),
    st("a1", 15, N),
];

static CHASTISE_1HS: [SeqStep; 16] = [
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, N),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 10, N),
    st("a2", 1, N),
    st("a2", 2, N),
    st("a2", 4, N),
    st("a2", 6, N),
    st("a2", 7, MELEE),
    st("a2", 8, N),
    st("a2", 10, N),
    st("a2", 11, N),
    st("a2", 13, N),
];

static CHARGE: [SeqStep; 15] = [
    st("rn", 0, MELEE),
    st("rn", 1, MELEE),
    st("rn", 2, MELEE),
    st("rn", 3, MELEE),
    st("rn", 4, MELEE),
    st("rn", 5, MELEE),
    st("rn", 6, MELEE),
    st("rn", 7, MELEE),
    st("a1", 1, N),
    st("a1", 4, N),
    st("a1", 5, SOUND),
    st("a1", 6, MELEE),
    st("a1", 8, N),
    st("a1", 10, N),
    st("a1", 12, N),
];

static DEFIANCE_1HS: [SeqStep; 6] = [
    st("a2", 5, N),
    st("a2", 6, N),
    st("a2", 7, MELEE),
    st("a2", 8, N),
    st("a2", 10, N),
    st("a2", 12, N),
];

static INFERNO: [SeqStep; 15] = [
    st("sc", 0, N),
    st("sc", 1, N),
    st("sc", 2, N),
    st("sc", 3, N),
    st("sc", 4, N),
    st("sc", 5, N),
    st("sc", 6, N),
    st("sc", 7, N),
    st("sc", 8, N),
    st("sc", 9, N),
    st("sc", 9, MELEE),
    st("sc", 10, N),
    st("sc", 11, N),
    st("sc", 12, N),
    st("sc", 13, N),
];

static STRAFE_BOW: [SeqStep; 13] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 6, MELEE),
    st("a1", 7, N),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
];

static STRAFE_XBW: [SeqStep; 20] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a1", 7, N),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, MELEE),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("a1", 15, N),
    st("a1", 16, N),
    st("a1", 17, N),
    st("a1", 18, N),
    st("a1", 19, N),
];

static IMPALE_1HT: [SeqStep; 21] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 1, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 2, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a1", 7, MELEE),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
];

static IMPALE_2HT: [SeqStep; 24] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 1, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 2, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a1", 7, N),
    st("a1", 8, N),
    st("a1", 9, MELEE),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("a1", 15, N),
    st("a1", 16, N),
    st("a1", 17, N),
];

static FEND_1HT: [SeqStep; 16] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, N),
    st("a2", 6, N),
    st("a1", 8, N),
    st("a2", 8, N),
    st("a1", 9, N),
    st("a2", 9, N),
    st("a1", 12, N),
    st("a2", 12, MELEE),
    st("a1", 13, N),
    st("a1", 14, N),
];

static FEND_2HT: [SeqStep; 16] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a2", 2, N),
    st("a1", 6, N),
    st("a2", 6, N),
    st("a1", 9, N),
    st("a2", 9, N),
    st("a1", 12, N),
    st("a2", 10, N),
    st("a1", 14, N),
    st("a2", 14, N),
    st("a1", 15, N),
    st("a1", 16, N),
    st("a1", 17, N),
    st("dt", 0, N),
];

static WHIRLWIND: [SeqStep; 8] = [
    st("a1", 0, N),
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 3, MELEE),
    st("a1", 3, N),
    st("a1", 4, N),
    st("a1", 5, N),
    st("a1", 6, MELEE),
];

static DOUBLE_SWING: [SeqStep; 17] = [
    st("a1", 1, N),
    st("a1", 2, N),
    st("a1", 4, N),
    st("a1", 6, N),
    st("a1", 7, MELEE),
    st("a1", 9, N),
    st("a1", 11, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("s3", 2, MELEE),
    st("s3", 4, N),
    st("s3", 5, N),
    st("s3", 6, N),
    st("s3", 8, N),
    st("s3", 9, N),
    st("s3", 10, N),
    st("s3", 11, N),
];

static LIGHTNING: [SeqStep; 19] = [
    st("sc", 0, N),
    st("sc", 1, N),
    st("sc", 3, N),
    st("sc", 4, N),
    st("sc", 5, N),
    st("sc", 7, N),
    st("sc", 8, N),
    st("sc", 9, MELEE),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 10, N),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 10, N),
    st("sc", 11, N),
    st("sc", 12, N),
    st("sc", 13, N),
];

static LEAP: [SeqStep; 15] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, MELEE),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 10, N),
    st("s1", 11, MELEE),
    st("s1", 12, N),
    st("s1", 13, N),
    st("s1", 14, MELEE),
];

static LEAP_ATTACK_HTH_ETC: [SeqStep; 22] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, MELEE),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 10, N),
    st("s1", 11, MELEE),
    st("s1", 12, N),
    st("s1", 13, N),
    st("s1", 14, N),
    st("a1", 5, N),
    st("a1", 6, MELEE),
    st("a1", 7, N),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
];

static LEAP_ATTACK_1HT_1HS: [SeqStep; 25] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, MELEE),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 10, N),
    st("s1", 11, MELEE),
    st("s1", 12, N),
    st("s1", 13, N),
    st("s1", 14, N),
    st("a1", 6, N),
    st("a1", 7, MELEE),
    st("a1", 8, N),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("a1", 15, N),
];

static LEAP_ATTACK_2HT_STF: [SeqStep; 28] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, MELEE),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 10, N),
    st("s1", 11, MELEE),
    st("s1", 12, N),
    st("s1", 13, N),
    st("s1", 14, N),
    st("a1", 6, N),
    st("a1", 7, N),
    st("a1", 8, N),
    st("a1", 9, MELEE),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("a1", 15, N),
    st("a1", 16, N),
    st("a1", 17, N),
    st("a1", 18, N),
];

static LEAP_ATTACK_2HS: [SeqStep; 27] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, MELEE),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 10, N),
    st("s1", 11, MELEE),
    st("s1", 12, N),
    st("s1", 13, N),
    st("s1", 14, N),
    st("a1", 6, N),
    st("a1", 7, N),
    st("a1", 8, MELEE),
    st("a1", 9, N),
    st("a1", 10, N),
    st("a1", 11, N),
    st("a1", 12, N),
    st("a1", 13, N),
    st("a1", 14, N),
    st("a1", 15, N),
    st("a1", 16, N),
    st("a1", 17, N),
];

static DOUBLE_THROW: [SeqStep; 12] = [
    st("th", 2, N),
    st("th", 3, N),
    st("th", 4, N),
    st("th", 6, N),
    st("th", 7, N),
    st("th", 8, MELEE),
    st("s3", 2, N),
    st("s3", 4, N),
    st("s3", 5, N),
    st("s3", 7, MELEE),
    st("s3", 8, N),
    st("s3", 10, N),
];

static DRAGON_CLAW_HTH_HT1: [SeqStep; 12] = [
    st("a2", 0, N),
    st("a2", 1, N),
    st("a2", 2, N),
    st("a2", 3, N),
    st("a2", 4, N),
    st("a2", 5, N),
    st("a2", 6, MELEE),
    st("a2", 7, N),
    st("a2", 8, N),
    st("a2", 9, N),
    st("a2", 10, N),
    st("a2", 11, N),
];

static DRAGON_CLAW_HT2: [SeqStep; 16] = [
    st("a2", 0, N),
    st("a2", 1, N),
    st("a2", 2, N),
    st("a2", 3, N),
    st("a2", 4, N),
    st("a2", 5, N),
    st("a2", 6, MELEE),
    st("s4", 3, N),
    st("s4", 4, N),
    st("s4", 5, N),
    st("s4", 6, MELEE),
    st("s4", 7, N),
    st("s4", 8, N),
    st("s4", 9, N),
    st("s4", 10, N),
    st("s4", 11, N),
];

static PROJECTION: [SeqStep; 19] = [
    st("sc", 0, N),
    st("sc", 1, N),
    st("sc", 2, N),
    st("sc", 3, N),
    st("sc", 4, N),
    st("sc", 5, N),
    st("sc", 6, N),
    st("sc", 7, MELEE),
    st("sc", 8, N),
    st("sc", 9, N),
    st("sc", 10, N),
    st("sc", 11, MELEE),
    st("sc", 13, N),
    st("sc", 14, N),
    st("sc", 15, N),
    st("sc", 16, N),
    st("sc", 17, N),
    st("sc", 18, MELEE),
    st("dt", 0, N),
];

static ARCTIC_BLAST: [SeqStep; 15] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, N),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, N),
    st("s1", 9, N),
    st("s1", 9, MELEE),
    st("s1", 10, N),
    st("s1", 11, N),
    st("s1", 12, N),
    st("s1", 13, N),
];

static DRAGON_TALON: [SeqStep; 20] = [
    st("kk", 0, N),
    st("kk", 1, N),
    st("kk", 2, N),
    st("kk", 3, N),
    st("kk", 4, MELEE),
    st("kk", 5, N),
    st("kk", 2, N),
    st("kk", 3, N),
    st("kk", 4, MELEE),
    st("kk", 5, N),
    st("kk", 2, N),
    st("kk", 3, N),
    st("kk", 4, N),
    st("kk", 5, MELEE),
    st("kk", 6, N),
    st("kk", 7, N),
    st("kk", 8, N),
    st("kk", 9, N),
    st("kk", 10, N),
    st("kk", 11, N),
];

static DRAGON_BREATH: [SeqStep; 17] = [
    st("s1", 0, N),
    st("s1", 1, N),
    st("s1", 2, N),
    st("s1", 3, N),
    st("s1", 4, N),
    st("s1", 5, N),
    st("s1", 6, N),
    st("s1", 7, N),
    st("s1", 8, MELEE),
    st("s1", 9, N),
    st("s1", 10, MELEE),
    st("s1", 11, N),
    st("s1", 12, MELEE),
    st("s1", 12, N),
    st("s1", 13, MELEE),
    st("s1", 13, N),
    st("s1", 14, N),
];

static DRAGON_FLIGHT: [SeqStep; 23] = [
    st("sc", 0, N),
    st("sc", 1, N),
    st("sc", 2, N),
    st("sc", 3, N),
    st("sc", 4, N),
    st("sc", 5, N),
    st("sc", 6, N),
    st("sc", 7, N),
    st("sc", 8, N),
    st("sc", 9, MELEE),
    st("kk", 0, N),
    st("kk", 1, N),
    st("kk", 2, N),
    st("kk", 3, N),
    st("kk", 4, MELEE),
    st("kk", 5, N),
    st("kk", 6, N),
    st("kk", 7, N),
    st("kk", 8, N),
    st("kk", 9, N),
    st("kk", 10, N),
    st("kk", 11, N),
    st("kk", 12, N),
];

static UNMORPH: [SeqStep; 16] = [
    st("s1", 15, N),
    st("s1", 14, N),
    st("s1", 13, N),
    st("s1", 12, N),
    st("s1", 11, N),
    st("s1", 10, N),
    st("s1", 9, N),
    st("s1", 8, N),
    st("s1", 7, N),
    st("s1", 6, N),
    st("s1", 5, N),
    st("s1", 4, N),
    st("s1", 3, N),
    st("s1", 2, MELEE),
    st("s1", 1, N),
    st("s1", 0, N),
];

static BLADE_FURY: [SeqStep; 19] = [
    st("sc", 0, N),
    st("sc", 1, N),
    st("sc", 2, N),
    st("sc", 3, N),
    st("sc", 4, N),
    st("sc", 5, N),
    st("sc", 6, N),
    st("sc", 7, N),
    st("sc", 8, N),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 9, N),
    st("sc", 10, MELEE),
    st("sc", 11, N),
    st("sc", 12, N),
    st("sc", 13, N),
    st("sc", 14, N),
    st("sc", 15, N),
    st("sc", 16, N),
];

/// Sequence numbers 1..=23 in order (`seqnum` 1 is index 0).
pub static PLAYER_SEQUENCES: [PlayerSequence; 23] = [
    PlayerSequence {
        seqnum: 1,
        name: "Jab",
        by_class: [
            Some(&HAND_TO_HAND1),
            Some(&JAB_1HT),
            Some(&JAB_2HT),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 2,
        name: "Sacrifice",
        by_class: [
            Some(&HAND_TO_HAND2),
            None,
            None,
            Some(&SACRIFICE_1HS),
            Some(&SACRIFICE_2HS),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 3,
        name: "Chastise",
        by_class: [
            Some(&HAND_TO_HAND2),
            None,
            None,
            Some(&CHASTISE_1HS),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 4,
        name: "Charge",
        by_class: [
            Some(&CHARGE),
            Some(&CHARGE),
            Some(&CHARGE),
            Some(&CHARGE),
            Some(&CHARGE),
            None,
            None,
            Some(&CHARGE),
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 5,
        name: "Defiance",
        by_class: [
            Some(&HAND_TO_HAND2),
            None,
            None,
            Some(&DEFIANCE_1HS),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 6,
        name: "Inferno",
        by_class: [
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
            Some(&INFERNO),
        ],
    },
    PlayerSequence {
        seqnum: 7,
        name: "Strafe",
        by_class: [
            Some(&HAND_TO_HAND1),
            None,
            None,
            None,
            None,
            Some(&STRAFE_BOW),
            Some(&STRAFE_XBW),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 8,
        name: "Impale",
        by_class: [
            Some(&HAND_TO_HAND1),
            Some(&IMPALE_1HT),
            Some(&IMPALE_2HT),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 9,
        name: "Fend",
        by_class: [
            Some(&HAND_TO_HAND1),
            Some(&FEND_1HT),
            Some(&FEND_2HT),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    },
    PlayerSequence {
        seqnum: 10,
        name: "Whirlwind",
        by_class: [
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            None,
            None,
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
            Some(&WHIRLWIND),
        ],
    },
    PlayerSequence {
        seqnum: 11,
        name: "Double Swing",
        by_class: [
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
            Some(&DOUBLE_SWING),
        ],
    },
    PlayerSequence {
        seqnum: 12,
        name: "Lightning",
        by_class: [
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
            Some(&LIGHTNING),
        ],
    },
    PlayerSequence {
        seqnum: 13,
        name: "Leap",
        by_class: [
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
            Some(&LEAP),
        ],
    },
    PlayerSequence {
        seqnum: 14,
        name: "Leap Attack",
        by_class: [
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_1HT_1HS),
            Some(&LEAP_ATTACK_2HT_STF),
            Some(&LEAP_ATTACK_1HT_1HS),
            Some(&LEAP_ATTACK_2HS),
            None,
            None,
            Some(&LEAP_ATTACK_2HT_STF),
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_HTH_ETC),
            Some(&LEAP_ATTACK_HTH_ETC),
        ],
    },
    PlayerSequence {
        seqnum: 15,
        name: "Double Throw",
        by_class: [
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
            Some(&DOUBLE_THROW),
        ],
    },
    PlayerSequence {
        seqnum: 16,
        name: "Dragon Claw",
        by_class: [
            Some(&DRAGON_CLAW_HTH_HT1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&DRAGON_CLAW_HTH_HT1),
            Some(&DRAGON_CLAW_HT2),
        ],
    },
    PlayerSequence {
        seqnum: 17,
        name: "Projection",
        by_class: [
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            Some(&PROJECTION),
            None,
            None,
            None,
            None,
            Some(&PROJECTION),
            Some(&PROJECTION),
        ],
    },
    PlayerSequence {
        seqnum: 18,
        name: "Arctic Blast",
        by_class: [
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
            Some(&ARCTIC_BLAST),
        ],
    },
    PlayerSequence {
        seqnum: 19,
        name: "Dragon Talon",
        by_class: [
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
            None,
            None,
            None,
            None,
            Some(&DRAGON_TALON),
            Some(&DRAGON_TALON),
        ],
    },
    PlayerSequence {
        seqnum: 20,
        name: "Dragon Breath",
        by_class: [
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
            None,
            None,
            None,
            None,
            None,
            Some(&DRAGON_BREATH),
            Some(&DRAGON_BREATH),
        ],
    },
    PlayerSequence {
        seqnum: 21,
        name: "Dragon Flight",
        by_class: [
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
            Some(&DRAGON_FLIGHT),
        ],
    },
    PlayerSequence {
        seqnum: 22,
        name: "Unmorph",
        by_class: [
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
            Some(&UNMORPH),
        ],
    },
    PlayerSequence {
        seqnum: 23,
        name: "Blade Fury",
        by_class: [
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
            Some(&BLADE_FURY),
        ],
    },
];

#[cfg(test)]
mod tests;
