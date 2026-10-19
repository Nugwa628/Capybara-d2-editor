//! The plan table of "Convert to palette": one row per used source colour,
//! worst error first, with how bad each match looks and whether its
//! override sits outside the allowed colours.

use capy_paint::remap::{MatchKind, RemapEntry, RemapPlan};

/// How visible a colour change is (OkLab distance x 100; lead decision:
/// <= 2 fine, > 5 visible, > 10 obvious).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Fine,
    Slight,
    Visible,
    Obvious,
}

pub const FINE_UP_TO: f32 = 2.0;
pub const VISIBLE_ABOVE: f32 = 5.0;
pub const OBVIOUS_ABOVE: f32 = 10.0;

pub fn severity(error: f32) -> Severity {
    if !error.is_finite() || error > OBVIOUS_ABOVE {
        Severity::Obvious
    } else if error > VISIBLE_ABOVE {
        Severity::Visible
    } else if error > FINE_UP_TO {
        Severity::Slight
    } else {
        Severity::Fine
    }
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Fine => "fine",
            Severity::Slight => "slight",
            Severity::Visible => "visible",
            Severity::Obvious => "obvious",
        }
    }

    /// RGB for the error cell.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Severity::Fine => [110, 200, 110],
            Severity::Slight => [200, 200, 110],
            Severity::Visible => [235, 160, 60],
            Severity::Obvious => [235, 80, 80],
        }
    }
}

pub fn how_label(how: MatchKind) -> &'static str {
    match how {
        MatchKind::Kept => "kept",
        MatchKind::Nearest => "nearest",
        MatchKind::Override => "override",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub entry: RemapEntry,
    pub severity: Severity,
    /// The index changes (`target != source`).
    pub changed: bool,
    /// An override whose target the allowed set excludes (honoured anyway).
    pub outside_allowed: bool,
}

/// The rows of `plan`, worst error first (ties: lower source index first).
/// `allowed(i)` tells whether target index `i` is in the allowed set.
pub fn rows(plan: &RemapPlan, allowed: impl Fn(u8) -> bool) -> Vec<Row> {
    let mut out: Vec<Row> = plan
        .entries
        .iter()
        .map(|e| Row {
            severity: severity(e.error),
            changed: e.target != e.source,
            outside_allowed: e.how == MatchKind::Override && !allowed(e.target),
            entry: e.clone(),
        })
        .collect();
    out.sort_by(|a, b| {
        b.entry
            .error
            .total_cmp(&a.entry.error)
            .then(a.entry.source.cmp(&b.entry.source))
    });
    out
}

/// Totals for the summary line and the toast.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Used colours.
    pub colours: usize,
    /// Used colours whose index changes.
    pub changed: usize,
    pub overrides: usize,
    /// Pixels whose index changes.
    pub changed_pixels: u64,
    pub mean: f32,
    pub worst: f32,
}

pub fn summary(plan: &RemapPlan) -> Summary {
    let changed: Vec<&RemapEntry> = plan
        .entries
        .iter()
        .filter(|e| e.target != e.source)
        .collect();
    Summary {
        colours: plan.entries.len(),
        changed: changed.len(),
        overrides: plan
            .entries
            .iter()
            .filter(|e| e.how == MatchKind::Override)
            .count(),
        changed_pixels: changed.iter().map(|e| e.pixels).sum(),
        mean: plan.mean_error,
        worst: plan.worst_error,
    }
}

impl Summary {
    pub fn text(&self) -> String {
        format!(
            "{} of {} colours change ({} pixels), mean error {:.2}, worst {:.1}{}",
            self.changed,
            self.colours,
            self.changed_pixels,
            self.mean,
            self.worst,
            if self.overrides > 0 {
                format!(", {} override(s)", self.overrides)
            } else {
                String::new()
            }
        )
    }
}

/// Source indices whose pixels a highlight marks: `changed` = the index
/// changes, `worst` = the match is visible or worse.
pub fn highlight_mask(plan: &RemapPlan, changed: bool, worst: bool) -> [bool; 256] {
    let mut mask = [false; 256];
    for e in &plan.entries {
        let c = changed && e.target != e.source;
        let w = worst && severity(e.error) >= Severity::Visible;
        mask[usize::from(e.source)] = c || w;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: u8, target: u8, error: f32, pixels: u64, how: MatchKind) -> RemapEntry {
        RemapEntry {
            source,
            target,
            source_rgb: [source; 3],
            target_rgb: [target; 3],
            error,
            pixels,
            how,
        }
    }

    fn plan(entries: Vec<RemapEntry>) -> RemapPlan {
        let mut table = [0u8; 256];
        for (i, t) in table.iter_mut().enumerate().skip(1) {
            *t = i as u8;
        }
        for e in &entries {
            table[usize::from(e.source)] = e.target;
        }
        RemapPlan {
            table,
            mean_error: 1.0,
            worst_error: entries.iter().map(|e| e.error).fold(0.0, f32::max),
            entries,
        }
    }

    #[test]
    fn severities_follow_the_thresholds() {
        assert_eq!(severity(0.0), Severity::Fine);
        assert_eq!(severity(2.0), Severity::Fine);
        assert_eq!(severity(2.1), Severity::Slight);
        assert_eq!(severity(5.0), Severity::Slight);
        assert_eq!(severity(5.1), Severity::Visible);
        assert_eq!(severity(10.5), Severity::Obvious);
        assert_eq!(severity(f32::NAN), Severity::Obvious);
    }

    #[test]
    fn rows_sort_worst_first_and_flag_overrides() {
        let p = plan(vec![
            entry(10, 10, 0.0, 50, MatchKind::Kept),
            entry(239, 164, 10.5, 7, MatchKind::Nearest),
            entry(230, 240, 3.0, 2, MatchKind::Override),
            entry(231, 40, 3.0, 9, MatchKind::Override),
        ]);
        let allowed = |i: u8| !(225..=254).contains(&i);
        let r = rows(&p, allowed);
        let order: Vec<u8> = r.iter().map(|r| r.entry.source).collect();
        assert_eq!(order, vec![239, 230, 231, 10], "worst first, ties by index");
        assert_eq!(r[0].severity, Severity::Obvious);
        assert!(
            r[1].outside_allowed,
            "override to 240 is outside shared colours"
        );
        assert!(!r[2].outside_allowed);
        assert!(!r[3].changed && r[0].changed);
        let s = summary(&p);
        assert_eq!(
            (s.colours, s.changed, s.overrides, s.changed_pixels),
            (4, 3, 2, 18)
        );
        assert!(s.text().contains("3 of 4 colours"), "{}", s.text());
        let m = highlight_mask(&p, false, true);
        assert!(m[239] && !m[230] && !m[10]);
        let m = highlight_mask(&p, true, false);
        assert!(m[239] && m[230] && m[231] && !m[10]);
    }
}
