//! Canonical X-Rite/Calibrite ColorChecker Classic 24 reference values.
//! sRGB-8 (D65), row-major from the dark-skin corner (patch 1) to black (patch 24).

use crate::color::srgb8_to_lab;

pub struct RefPatch {
    pub name: &'static str,
    pub srgb: [u8; 3],
}

pub const CLASSIC24: [RefPatch; 24] = [
    RefPatch {
        name: "Dark Skin",
        srgb: [115, 82, 68],
    },
    RefPatch {
        name: "Light Skin",
        srgb: [194, 150, 130],
    },
    RefPatch {
        name: "Blue Sky",
        srgb: [98, 122, 157],
    },
    RefPatch {
        name: "Foliage",
        srgb: [87, 108, 67],
    },
    RefPatch {
        name: "Blue Flower",
        srgb: [133, 128, 177],
    },
    RefPatch {
        name: "Bluish Green",
        srgb: [103, 189, 170],
    },
    RefPatch {
        name: "Orange",
        srgb: [214, 126, 44],
    },
    RefPatch {
        name: "Purplish Blue",
        srgb: [80, 91, 166],
    },
    RefPatch {
        name: "Moderate Red",
        srgb: [193, 90, 99],
    },
    RefPatch {
        name: "Purple",
        srgb: [94, 60, 108],
    },
    RefPatch {
        name: "Yellow Green",
        srgb: [157, 188, 64],
    },
    RefPatch {
        name: "Orange Yellow",
        srgb: [224, 163, 46],
    },
    RefPatch {
        name: "Blue",
        srgb: [56, 61, 150],
    },
    RefPatch {
        name: "Green",
        srgb: [70, 148, 73],
    },
    RefPatch {
        name: "Red",
        srgb: [175, 54, 60],
    },
    RefPatch {
        name: "Yellow",
        srgb: [231, 199, 31],
    },
    RefPatch {
        name: "Magenta",
        srgb: [187, 86, 149],
    },
    RefPatch {
        name: "Cyan",
        srgb: [8, 133, 161],
    },
    RefPatch {
        name: "White",
        srgb: [243, 243, 242],
    },
    RefPatch {
        name: "Neutral 8",
        srgb: [200, 200, 200],
    },
    RefPatch {
        name: "Neutral 6.5",
        srgb: [160, 160, 160],
    },
    RefPatch {
        name: "Neutral 5",
        srgb: [122, 122, 121],
    },
    RefPatch {
        name: "Neutral 3.5",
        srgb: [85, 85, 85],
    },
    RefPatch {
        name: "Black",
        srgb: [52, 52, 52],
    },
];

/// The 24 reference patches as Lab (D65), row-major.
pub fn classic24_lab() -> [[f32; 3]; 24] {
    let mut out = [[0.0f32; 3]; 24];
    for (i, p) in CLASSIC24.iter().enumerate() {
        out[i] = srgb8_to_lab(p.srgb);
    }
    out
}

/// Indices (0-based) of the six neutral patches (white → black), used for WB fitting.
pub const NEUTRAL_INDICES: [usize; 6] = [18, 19, 20, 21, 22, 23];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::delta_e_2000;

    #[test]
    fn table_has_24_patches_named() {
        assert_eq!(CLASSIC24.len(), 24);
        assert_eq!(CLASSIC24[0].name, "Dark Skin");
        assert_eq!(CLASSIC24[23].name, "Black");
    }

    #[test]
    fn lab_sanity_white_black_neutral() {
        let lab = classic24_lab();
        // Patch 19 = White (bright), patch 24 = Black (dark).
        assert!(lab[18][0] > 94.0, "white L*={}", lab[18][0]);
        assert!(lab[23][0] < 25.0, "black L*={}", lab[23][0]);
        // Neutrals (19..24) are near-achromatic.
        for (i, patch) in lab.iter().enumerate().take(24).skip(18) {
            let c = (patch[1].powi(2) + patch[2].powi(2)).sqrt();
            assert!(c < 3.0, "patch {} chroma {}", i + 1, c);
        }
        // Each patch differs from its neighbor by a visible amount.
        assert!(delta_e_2000(lab[0], lab[1]) > 5.0);
    }
}
