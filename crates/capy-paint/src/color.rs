//! sRGB <-> linear light, built from `+ - * /` only so that results are
//! bit-identical on every platform (no libm `pow`).

/// `a^(1/n)` for `a` in `[0, 1]` by Newton iteration.
pub(crate) fn nth_root(a: f64, n: u32) -> f64 {
    if a <= 0.0 {
        return 0.0;
    }
    let nf = f64::from(n);
    let mut r = 1.0;
    for _ in 0..80 {
        let mut p = 1.0;
        for _ in 1..n {
            p *= r;
        }
        let next = ((nf - 1.0) * r + a / p) / nf;
        if (next - r).abs() < 1e-15 {
            r = next;
            break;
        }
        r = next;
    }
    r
}

/// sRGB-encoded value in `[0, 1]` to linear light.
pub(crate) fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        // ((c + 0.055) / 1.055)^2.4 = x^2 * x^(2/5), with x the base.
        let x = (c + 0.055) / 1.055;
        x * x * nth_root(x * x, 5)
    }
}

/// Linear light in `[0, 1]` to sRGB-encoded.
pub(crate) fn linear_to_srgb(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        // 1.055 * l^(1/2.4) - 0.055, with l^(5/12) = (l^5)^(1/12).
        let l5 = l * l * l * l * l;
        1.055 * nth_root(l5, 12) - 0.055
    }
}

/// Lookup table: sRGB byte to linear light.
pub(crate) struct LinearTable([f64; 256]);

impl LinearTable {
    pub(crate) fn new() -> LinearTable {
        let mut t = [0.0; 256];
        for (i, v) in t.iter_mut().enumerate() {
            *v = srgb_to_linear(i as f64 / 255.0);
        }
        LinearTable(t)
    }

    pub(crate) fn to_linear(&self, rgb: [u8; 3]) -> [f64; 3] {
        rgb.map(|c| self.0[usize::from(c)])
    }

    /// Nearest sRGB byte triple to a linear colour (components clamped).
    pub(crate) fn to_srgb8(&self, lin: [f64; 3]) -> [u8; 3] {
        lin.map(|l| {
            let encoded = linear_to_srgb(l) * 255.0;
            (encoded + 0.5).clamp(0.0, 255.0) as u8
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_bytes() {
        let t = LinearTable::new();
        for c in 0..=255u8 {
            assert_eq!(t.to_srgb8(t.to_linear([c, c, c])), [c, c, c]);
        }
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-12);
        assert!((srgb_to_linear(0.5) - 0.214_041_14).abs() < 1e-7);
    }
}
