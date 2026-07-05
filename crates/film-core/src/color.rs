//! Color-space conversions and ΔE2000 for the benchmark harness.
//! Everything is sRGB/D65; no chromatic adaptation (both sides are D65).

/// sRGB EOTF: gamma-encoded [0,1] → linear [0,1].
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear sRGB (D65 primaries) → CIE XYZ (D65).
pub fn linear_rgb_to_xyz(rgb: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = rgb;
    [
        0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b,
        0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b,
        0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b,
    ]
}

/// CIE XYZ (D65) → CIE L*a*b* (D65 white point).
pub fn xyz_to_lab(xyz: [f32; 3]) -> [f32; 3] {
    const XN: f32 = 0.950_47;
    const YN: f32 = 1.0;
    const ZN: f32 = 1.088_83;
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let fx = f(xyz[0] / XN);
    let fy = f(xyz[1] / YN);
    let fz = f(xyz[2] / ZN);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// sRGB 8-bit → Lab (D65). Used for the reference ColorChecker table.
pub fn srgb8_to_lab(rgb: [u8; 3]) -> [f32; 3] {
    let lin = [
        srgb_to_linear(rgb[0] as f32 / 255.0),
        srgb_to_linear(rgb[1] as f32 / 255.0),
        srgb_to_linear(rgb[2] as f32 / 255.0),
    ];
    xyz_to_lab(linear_rgb_to_xyz(lin))
}

/// Display sRGB [0,1] (engine output) → Lab (D65). Clamps out-of-range first.
pub fn srgbf_to_lab(rgb: [f32; 3]) -> [f32; 3] {
    let lin = [
        srgb_to_linear(rgb[0].clamp(0.0, 1.0)),
        srgb_to_linear(rgb[1].clamp(0.0, 1.0)),
        srgb_to_linear(rgb[2].clamp(0.0, 1.0)),
    ];
    xyz_to_lab(linear_rgb_to_xyz(lin))
}

#[inline]
fn hue_deg(b: f64, ap: f64) -> f64 {
    if b == 0.0 && ap == 0.0 {
        0.0
    } else {
        let mut h = b.atan2(ap).to_degrees();
        if h < 0.0 {
            h += 360.0;
        }
        h
    }
}

/// CIEDE2000 color difference (kL=kC=kH=1). Reference: Sharma, Wu & Dalal (2005).
pub fn delta_e_2000(lab1: [f32; 3], lab2: [f32; 3]) -> f32 {
    let (l1, a1, b1) = (lab1[0] as f64, lab1[1] as f64, lab1[2] as f64);
    let (l2, a2, b2) = (lab2[0] as f64, lab2[1] as f64, lab2[2] as f64);
    let pow7 = |x: f64| x.powi(7);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cbar = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (pow7(cbar) / (pow7(cbar) + pow7(25.0))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let h1p = hue_deg(b1, a1p);
    let h2p = hue_deg(b2, a2p);

    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else {
        let mut dh = h2p - h1p;
        if dh > 180.0 {
            dh -= 360.0;
        } else if dh < -180.0 {
            dh += 360.0;
        }
        2.0 * (c1p * c2p).sqrt() * (dh.to_radians() / 2.0).sin()
    };

    let lbar = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() > 180.0 {
        let h = h1p + h2p;
        if h < 360.0 {
            (h + 360.0) / 2.0
        } else {
            (h - 360.0) / 2.0
        }
    } else {
        (h1p + h2p) / 2.0
    };

    let t = 1.0 - 0.17 * (hbarp - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hbarp).to_radians().cos()
        + 0.32 * (3.0 * hbarp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hbarp - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-(((hbarp - 275.0) / 25.0).powi(2))).exp();
    let rc = 2.0 * (pow7(cbarp) / (pow7(cbarp) + pow7(25.0))).sqrt();
    let sl = 1.0 + (0.015 * (lbar - 50.0).powi(2)) / (20.0 + (lbar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -rc * (2.0 * dtheta.to_radians()).sin();

    let de = ((dlp / sl).powi(2)
        + (dcp / sc).powi(2)
        + (dhp / sh).powi(2)
        + rt * (dcp / sc) * (dhp / sh))
        .sqrt();
    de as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_endpoints_to_lab() {
        // White -> L*100, neutral a/b ~ 0. Black -> 0,0,0.
        let w = srgb8_to_lab([255, 255, 255]);
        assert!((w[0] - 100.0).abs() < 0.2, "L*={}", w[0]);
        assert!(w[1].abs() < 0.2 && w[2].abs() < 0.2);
        let k = srgb8_to_lab([0, 0, 0]);
        assert!(k[0].abs() < 0.01 && k[1].abs() < 0.01 && k[2].abs() < 0.01);
    }

    #[test]
    fn srgb_midgray_lightness() {
        // sRGB 119/255 is ~middle L*; should land near L*50.
        let g = srgb8_to_lab([119, 119, 119]);
        assert!((g[0] - 50.0).abs() < 1.0, "L*={}", g[0]);
        assert!(g[1].abs() < 0.5 && g[2].abs() < 0.5);
    }

    #[test]
    fn delta_e_2000_sharma_vectors() {
        // Sharma/Wu/Dalal reference pairs (tolerance 1e-2).
        let cases = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, -1.3802, -84.2814], [50.0, 0.0, -82.7485], 1.0000),
            ([50.0, 2.5, 0.0], [50.0, 3.2972, 0.0], 1.0000),
            (
                [60.2574, -34.0099, 36.2677],
                [60.4626, -34.1751, 39.4387],
                1.2644,
            ),
        ];
        for (a, b, want) in cases {
            let got = delta_e_2000(a, b);
            assert!(
                (got - want).abs() < 1e-2,
                "ΔE00 {a:?} vs {b:?}: got {got}, want {want}"
            );
        }
        // Identity is zero.
        assert!(delta_e_2000([50.0, 2.5, 0.0], [50.0, 2.5, 0.0]).abs() < 1e-4);
    }
}
