use crate::wedge::{load_manifest, load_reference, target_lstar, RefPatch, WedgeFrame};
use film_core::calibrate::sample_base_clearfilm;
use film_core::chart::{sample_grid, sampling_overlay, GridSpec};
use film_core::decode::decode_raw;
use film_core::engine::{invert_image, InversionParams, Mode};
use film_core::tone::{
    ev_weight, fit_tone, output_lstar, transfer_metrics, FitMode, TonePoint, Transfer,
};
use std::path::Path;

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Sample one frame's 100 patches (raw negative values), rank-pair to the reference
/// EVs, attach base offset → absolute-EV TonePoints. `ref_sorted` is the reference
/// patches sorted ascending by EV; `value_max` is the global brightest value.
fn frame_points(
    f: &WedgeFrame,
    dir: &str,
    ref_sorted: &[RefPatch],
    value_max: f32,
    out_dir: &str,
) -> Result<Vec<TonePoint>, String> {
    let path = Path::new(dir).join(&f.file);
    let neg = decode_raw(&path, false).map_err(|e| format!("decode {}: {e}", path.display()))?;
    let base = sample_base_clearfilm(&neg, 0.92, 0.95);
    let spec = GridSpec {
        cols: 10,
        rows: 10,
        inset: 0.45,
    };
    let scans = sample_grid(&neg, &f.corners, &spec, 0.2); // 100 raw negative patches

    // Overlay (verification) from the neutral inversion of this frame.
    let pos = invert_image(
        &neg,
        &InversionParams {
            base,
            ..Default::default()
        },
        Mode::D,
    );
    let ov = sampling_overlay(&pos, &f.corners, &spec, 1600);
    let ovp = format!("{out_dir}/overlay_{}.png", sanitize(&f.file));
    ov.save(&ovp).map_err(|e| format!("save {ovp}: {e}"))?;

    // Rank-pair: sort sampled patches by brightness (luma of the inverted positive is
    // monotone in scene EV); pair rank-wise with EV-ascending reference patches.
    let pos_patches = sample_grid(&pos, &f.corners, &spec, 0.2);
    let mut order: Vec<usize> = (0..scans.len()).collect();
    let luma = |p: [f32; 3]| 0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2];
    order.sort_by(|&a, &b| {
        luma(pos_patches[a])
            .partial_cmp(&luma(pos_patches[b]))
            .unwrap()
    });
    // order[k] = index of the k-th darkest patch; ref_sorted[k] = k-th lowest EV.

    let mut pts = Vec::with_capacity(scans.len());
    for (k, &i) in order.iter().enumerate() {
        let rp = ref_sorted[k.min(ref_sorted.len() - 1)];
        let abs_ev = rp.ev + f.base_ev;
        pts.push(TonePoint {
            scan: scans[i],
            base,
            target_l: target_lstar(rp.value, value_max),
            weight: ev_weight(abs_ev),
            abs_ev,
        });
    }
    Ok(pts)
}

pub fn run(manifest_path: &str, out_dir: &str) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir {out_dir}: {e}"))?;
    let m = load_manifest(manifest_path)?;
    let mut reference = load_reference(&m.reference)?;
    reference.sort_by(|a, b| a.ev.partial_cmp(&b.ev).unwrap());
    let value_max = reference.iter().map(|p| p.value).fold(0.0f32, f32::max);

    let baseline_scale = 1.0 / 1.5; // engine default d_max
    let mut json = String::from("{\n  \"frames\": [\n");
    let mut csv = String::from("frame,abs_ev,target_l,baseline_l,fit_l,weight\n");
    eprintln!("=== tone calibration (per-frame fidelity vs digital-SDR reference) ===");

    for (fi, f) in m.frames.iter().enumerate() {
        let points = frame_points(f, &m.dir, &reference, value_max, out_dir)?;
        let base_m = transfer_metrics(&points, baseline_scale, &Transfer::default_filmic());
        let fits = [
            ("scale_only", fit_tone(&points, FitMode::ScaleOnly)),
            ("scale_curve", fit_tone(&points, FitMode::ScaleCurve)),
            ("gamma", fit_tone(&points, FitMode::Gamma)),
            ("gamma_shoulder", fit_tone(&points, FitMode::GammaShoulder)),
        ];

        // JSON object for this frame
        json.push_str(&format!(
            "    {{ \"file\": {:?}, \"base_ev\": {:.1}, \"baseline\": {{ \"d_max\": 1.5, \"rms_dl\": {:.3}, \"max_dl\": {:.3}, \"frac_within5\": {:.3}, \"monotonic\": {} }}, \"fits\": [\n",
            f.file, f.base_ev, base_m.rms_dl, base_m.max_dl, base_m.frac_within5, base_m.monotonic
        ));
        for (i, (name, fr)) in fits.iter().enumerate() {
            let curve = match fr.transfer {
                Transfer::Filmic { k, pivot, white_t } => format!(
                    "\"filmic\", \"k\": {k:.3}, \"pivot\": {pivot:.3}, \"white_t\": {white_t:.3}"
                ),
                Transfer::Gamma { gamma } => format!("\"gamma\", \"gamma\": {gamma:.3}"),
                Transfer::GammaShoulder { gamma, knee } => {
                    format!("\"gamma_shoulder\", \"gamma\": {gamma:.3}, \"knee\": {knee:.3}")
                }
            };
            json.push_str(&format!(
                "      {{ \"mode\": {:?}, \"residual_rms\": {:.3}, \"recommended_d_max\": {:.3}, \"transfer\": {} }}{}\n",
                name, fr.residual_rms, 1.0 / fr.scale, curve,
                if i + 1 < fits.len() { "," } else { "" }
            ));
        }
        json.push_str(&format!(
            "    ] }}{}\n",
            if fi + 1 < m.frames.len() { "," } else { "" }
        ));

        // CSV: best fit for this frame
        let best = &fits
            .iter()
            .min_by(|a, b| a.1.residual_rms.partial_cmp(&b.1.residual_rms).unwrap())
            .unwrap()
            .1;
        let mut order: Vec<usize> = (0..points.len()).collect();
        order.sort_by(|&a, &b| points[a].abs_ev.partial_cmp(&points[b].abs_ev).unwrap());
        for &i in &order {
            let p = &points[i];
            let bl = output_lstar(p.scan, p.base, baseline_scale, &Transfer::default_filmic());
            let fl = output_lstar(p.scan, p.base, best.scale, &best.transfer);
            csv.push_str(&format!(
                "{},{:.3},{:.2},{:.2},{:.2},{:.3}\n",
                sanitize(&f.file),
                p.abs_ev,
                p.target_l,
                bl,
                fl,
                p.weight
            ));
        }

        // headline per frame
        eprintln!(
            "  {} (+{:.0} EV): baseline rms \u{0394}L* {:.1} (within5 {:.0}%, monotonic {})",
            f.file,
            f.base_ev,
            base_m.rms_dl,
            base_m.frac_within5 * 100.0,
            base_m.monotonic
        );
        for (name, fr) in &fits {
            eprintln!(
                "      fit {name:<11}: residual \u{0394}L* {:.1}  (recommended d_max {:.2})",
                fr.residual_rms,
                1.0 / fr.scale
            );
        }
    }

    json.push_str("  ]\n}\n");
    std::fs::write(format!("{out_dir}/tone_report.json"), json)
        .map_err(|e| format!("write report: {e}"))?;
    std::fs::write(format!("{out_dir}/transfer_curve.csv"), csv)
        .map_err(|e| format!("write csv: {e}"))?;
    eprintln!("outputs in {out_dir}/ (tone_report.json, transfer_curve.csv, overlay_*.png)");
    Ok(())
}
