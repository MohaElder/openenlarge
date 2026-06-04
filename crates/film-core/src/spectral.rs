//! Physical forward model: film dye concentrations → sensor RGB.
//!
//! D(λ) = D_min(λ) + Σ_{j=C,M,Y} c_j · D_j(λ)        (Beer-Lambert)
//! I_i  = Σ_λ L(λ) · S_i(λ) · 10^(−D(λ))             (sensor readout)

/// Which film stock's dye data to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stock {
    Portra400,
    FujiC200,
}

/// Spectral curves on a shared wavelength grid (nm). All Vecs are the same length.
#[derive(Debug, Clone)]
pub struct SpectralData {
    pub wavelengths: Vec<f32>,
    /// Per-unit-concentration spectral density of the C, M, Y dyes.
    pub dye: [Vec<f32>; 3],
    /// Minimum (base) spectral density.
    pub d_min: Vec<f32>,
    /// Light-source spectral power L(λ).
    pub illuminant: Vec<f32>,
    /// Sensor spectral sensitivity S_i(λ) for R, G, B.
    pub sensor: [Vec<f32>; 3],
}

/// The standard working grid: 380..=730 nm at 5 nm steps (71 samples).
pub fn grid_380_730_5() -> Vec<f32> {
    (0..=70).map(|k| 380.0 + 5.0 * k as f32).collect()
}

/// Representative analytic camera sensor: unit-peak Gaussians (R 600/σ30,
/// G 540/σ30, B 460/σ30). Open stand-in; replace with fitted per-camera SS later.
pub fn analytic_sensor(wavelengths: &[f32]) -> [Vec<f32>; 3] {
    let g = |center: f32, sigma: f32| -> Vec<f32> {
        wavelengths
            .iter()
            .map(|&w| (-0.5 * ((w - center) / sigma).powi(2)).exp())
            .collect()
    };
    [g(600.0, 30.0), g(540.0, 30.0), g(460.0, 30.0)]
}

impl SpectralData {
    /// Forward model: concentrations `c = [c_C, c_M, c_Y]` → sensor RGB readout.
    pub fn simulate(&self, c: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for k in 0..self.wavelengths.len() {
            let d = self.d_min[k] + c[0] * self.dye[0][k] + c[1] * self.dye[1][k] + c[2] * self.dye[2][k];
            let lt = self.illuminant[k] * 10f32.powf(-d);
            for i in 0..3 {
                out[i] += lt * self.sensor[i][k];
            }
        }
        out
    }

    /// Clear-film response (c = 0): the per-channel base for density normalization.
    pub fn base(&self) -> [f32; 3] {
        self.simulate([0.0, 0.0, 0.0])
    }
}

/// Deterministic synthetic spectral data with deliberately OVERLAPPING dyes
/// (primary band + a secondary lobe), so the recovered M_post is non-trivial.
/// Used only by tests — keeps the fit math independent of the bundled CSVs.
#[cfg(test)]
pub(crate) fn synthetic_overlapping() -> SpectralData {
    let w = grid_380_730_5();
    let g = |center: f32, sigma: f32, amp: f32| -> Vec<f32> {
        w.iter()
            .map(|&x| amp * (-0.5 * ((x - center) / sigma).powi(2)).exp())
            .collect()
    };
    let add = |a: Vec<f32>, b: Vec<f32>| -> Vec<f32> {
        a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
    };
    // Cyan absorbs red (primary 650) + unwanted green (secondary 560).
    let dye_c = add(g(650.0, 50.0, 1.0), g(560.0, 40.0, 0.2));
    // Magenta absorbs green (550) + unwanted blue (460).
    let dye_m = add(g(550.0, 50.0, 1.0), g(460.0, 40.0, 0.2));
    // Yellow absorbs blue (450) + unwanted green (560).
    let dye_y = add(g(450.0, 50.0, 1.0), g(560.0, 40.0, 0.2));
    let d_min = w.iter().map(|_| 0.1f32).collect();
    let illuminant = w.iter().map(|_| 1.0f32).collect(); // equal-energy for the synthetic test
    let sensor = analytic_sensor(&w);
    SpectralData { wavelengths: w, dye: [dye_c, dye_m, dye_y], d_min, illuminant, sensor }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_71_samples_from_380_to_730() {
        let w = grid_380_730_5();
        assert_eq!(w.len(), 71);
        assert_eq!(w[0], 380.0);
        assert_eq!(*w.last().unwrap(), 730.0);
    }

    #[test]
    fn sensor_peaks_are_unit_at_centers() {
        let w = grid_380_730_5();
        let s = analytic_sensor(&w);
        // index of 600nm = (600-380)/5 = 44; 540 -> 32; 460 -> 16
        assert!((s[0][44] - 1.0).abs() < 1e-6);
        assert!((s[1][32] - 1.0).abs() < 1e-6);
        assert!((s[2][16] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn simulate_zero_conc_equals_base_and_is_brightest() {
        let d = synthetic_overlapping();
        let base = d.base();
        let dyed = d.simulate([1.0, 1.0, 1.0]);
        for i in 0..3 {
            assert!(base[i] > 0.0);
            assert!(dyed[i] < base[i], "channel {i}: dyed {} !< base {}", dyed[i], base[i]);
        }
    }

    #[test]
    fn cyan_dye_darkens_red_channel_most() {
        let d = synthetic_overlapping();
        let base = d.base();
        let cyan = d.simulate([1.5, 0.0, 0.0]);
        let red_drop = (base[0] - cyan[0]) / base[0];
        let blue_drop = (base[2] - cyan[2]) / base[2];
        assert!(red_drop > blue_drop, "red_drop {red_drop} !> blue_drop {blue_drop}");
    }
}
