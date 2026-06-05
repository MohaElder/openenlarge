//! Monotone cubic (Fritsch–Carlson) interpolation for tone curves. Mirrors
//! `app/src/lib/develop/curve.ts` — keep the two numerically identical so the CPU
//! finish and the GPU LUT produce matching results.

pub const LUT_SIZE: usize = 256;

#[inline]
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

struct Prepared {
    xs: Vec<f32>,
    ys: Vec<f32>,
    m: Vec<f32>, // tangents
}

/// Sort, dedupe by x, and compute monotone Hermite tangents.
fn prepare(points: &[[f32; 2]]) -> Prepared {
    let mut sorted: Vec<[f32; 2]> = points.to_vec();
    sorted.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));

    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for p in &sorted {
        if !xs.is_empty() && (p[0] - xs[xs.len() - 1]).abs() < 1e-6 {
            let last = ys.len() - 1;
            ys[last] = p[1]; // duplicate x: last point wins
        } else {
            xs.push(p[0]);
            ys.push(p[1]);
        }
    }
    let n = xs.len();
    if n <= 1 {
        return Prepared { xs, ys, m: vec![0.0] };
    }

    let mut d = vec![0.0f32; n - 1]; // secant slopes
    for k in 0..n - 1 {
        d[k] = (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]);
    }

    let mut m = vec![0.0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = (d[k - 1] + d[k]) / 2.0;
    }

    // Fritsch–Carlson monotonicity filter.
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let a = m[k] / d[k];
            let b = m[k + 1] / d[k];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[k] = t * a * d[k];
                m[k + 1] = t * b * d[k];
            }
        }
    }
    Prepared { xs, ys, m }
}

fn eval_prepared(p: &Prepared, x: f32) -> f32 {
    let (xs, ys, m) = (&p.xs, &p.ys, &p.m);
    let n = xs.len();
    if x <= xs[0] {
        return clamp01(ys[0]);
    }
    if x >= xs[n - 1] {
        return clamp01(ys[n - 1]);
    }
    let mut k = 0;
    while k < n - 1 && x > xs[k + 1] {
        k += 1;
    }
    let h = xs[k + 1] - xs[k];
    let t = (x - xs[k]) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    let y = h00 * ys[k] + h10 * h * m[k] + h01 * ys[k + 1] + h11 * h * m[k + 1];
    clamp01(y)
}

/// Sample the monotone-cubic curve through `points` at x ∈ [0,1] → [0,1].
pub fn sample_curve(points: &[[f32; 2]], x: f32) -> f32 {
    eval_prepared(&prepare(points), x)
}

/// Build a LUT_SIZE-entry table (output 0..1) sampling the curve at i/(N−1).
pub fn curve_lut(points: &[[f32; 2]]) -> [f32; LUT_SIZE] {
    let p = prepare(points);
    let mut out = [0.0f32; LUT_SIZE];
    for (i, o) in out.iter_mut().enumerate() {
        *o = eval_prepared(&p, i as f32 / (LUT_SIZE - 1) as f32);
    }
    out
}

/// Linear lookup into a 0..1 LUT at x ∈ [0,1].
pub fn sample_lut(lut: &[f32], x: f32) -> f32 {
    let n = lut.len();
    let f = clamp01(x) * (n - 1) as f32;
    let i = f.floor() as usize;
    if i >= n - 1 {
        return lut[n - 1];
    }
    let t = f - i as f32;
    lut[i] * (1.0 - t) + lut[i + 1] * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 2]; 2] = [[0.0, 0.0], [1.0, 1.0]];

    #[test]
    fn identity_returns_input() {
        for x in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            assert!((sample_curve(&IDENTITY, x) - x).abs() < 1e-5, "x={x}");
        }
    }

    #[test]
    fn clamps_output() {
        let steep = [[0.0, 0.0], [0.5, 1.0], [1.0, 1.0]];
        for x in [0.0, 0.5, 1.0] {
            let y = sample_curve(&steep, x);
            assert!((0.0..=1.0).contains(&y), "y={y}");
        }
    }

    #[test]
    fn endpoints_flat_extrapolate() {
        let lifted = [[0.0, 0.1], [1.0, 0.9]];
        assert!((sample_curve(&lifted, 0.0) - 0.1).abs() < 1e-5);
        assert!((sample_curve(&lifted, 1.0) - 0.9).abs() < 1e-5);
        assert!((sample_curve(&lifted, -1.0) - 0.1).abs() < 1e-5);
        assert!((sample_curve(&lifted, 2.0) - 0.9).abs() < 1e-5);
    }

    #[test]
    fn identity_lut_is_ramp() {
        let lut = curve_lut(&IDENTITY);
        assert!((lut[0]).abs() < 1e-5);
        assert!((lut[LUT_SIZE - 1] - 1.0).abs() < 1e-5);
        assert!((lut[128] - 128.0 / 255.0).abs() < 1e-4);
    }

    #[test]
    fn lut_stays_monotone_with_midtone_lift() {
        let lut = curve_lut(&[[0.0, 0.0], [0.25, 0.45], [0.75, 0.6], [1.0, 1.0]]);
        for i in 1..lut.len() {
            assert!(lut[i] >= lut[i - 1] - 1e-6, "non-monotone at {i}");
        }
    }

    #[test]
    fn sample_lut_interpolates_ramp() {
        let lut = curve_lut(&IDENTITY);
        assert!((sample_lut(&lut, 0.5) - 0.5).abs() < 1e-3);
        assert!((sample_lut(&lut, 0.0)).abs() < 1e-5);
        assert!((sample_lut(&lut, 1.0) - 1.0).abs() < 1e-5);
    }
}
