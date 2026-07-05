use crate::manifest::{load, Frame};
use film_core::bench::{score_color, score_tone};
use film_core::calibrate::sample_base_clearfilm;
use film_core::chart::{sampling_overlay, GridSpec};
use film_core::decode::decode_raw;
use film_core::engine::{invert_image, InversionParams, Mode};
use std::path::Path;

fn decode(dir: &str, file: &str) -> film_core::Image {
    let path = Path::new(dir).join(file);
    decode_raw(&path, false).unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
}

pub fn run(manifest_path: &str, out_dir: &str) -> Result<(), String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("mkdir {out_dir}: {e}"))?;
    let m = load(manifest_path)?;

    if m.chart != "colorchecker24" {
        return Err(format!(
            "unsupported chart {:?} (this harness only supports \"colorchecker24\")",
            m.chart
        ));
    }

    // Per-roll film base from the d_min frame. A clear leader shot on a lightbox
    // has a blown-white surround brighter than the orange mask, so a plain
    // high-percentile sampler returns ~[1,1,1] (no mask compensation → blue cast).
    // `sample_base_clearfilm` rejects the clipped surround and recovers the mask.
    let base = m
        .frames
        .iter()
        .find(|f| f.role == "d_min")
        .map(|f| {
            let img = decode(&m.dir, &f.file);
            sample_base_clearfilm(&img, 0.92, 0.95)
        })
        .unwrap_or([1.0, 1.0, 1.0]);

    let mut json = String::from("{\n");
    json.push_str(&format!("  \"roll\": {:?},\n", m.roll));
    json.push_str(&format!(
        "  \"base\": [{}, {}, {}],\n",
        base[0], base[1], base[2]
    ));
    json.push_str("  \"color\": [\n");

    let mut summary = Vec::new();

    let color_frames: Vec<&Frame> = m.frames.iter().filter(|f| f.role == "color").collect();
    for (idx, f) in color_frames.iter().enumerate() {
        let corners = f
            .corners
            .ok_or_else(|| format!("color frame {} missing corners", f.file))?;
        let neg = decode(&m.dir, &f.file);
        let rep = score_color(&neg, base, &corners);

        // Overlay for human verification.
        let positive = invert_image(
            &neg,
            &InversionParams {
                base,
                ..Default::default()
            },
            Mode::D,
        );
        let spec = GridSpec {
            cols: 6,
            rows: 4,
            inset: 0.5,
        };
        let ov = sampling_overlay(&positive, &corners, &spec, 1400);
        let ov_path = format!("{out_dir}/overlay_{}.png", sanitize(&f.file));
        ov.save(&ov_path)
            .map_err(|e| format!("save {ov_path}: {e}"))?;

        json.push_str(&format!(
            "    {{ \"file\": {:?}, \"neutralized_mean\": {:.4}, \"neutralized_max\": {:.4}, \"chroma_mean\": {:.4}, \"as_shipped_mean\": {:.4}, \"flags\": {} }}{}\n",
            f.file,
            rep.neutralized.mean,
            rep.neutralized.max,
            rep.neutralized_chroma_only.mean,
            rep.as_shipped.mean,
            json_str_array(&f.flags),
            if idx + 1 < color_frames.len() { "," } else { "" }
        ));
        summary.push(format!(
            "  {}: neutralized ΔE mean {:.2} (chroma {:.2}, max {:.2}) | as-shipped {:.2}",
            f.file,
            rep.neutralized.mean,
            rep.neutralized_chroma_only.mean,
            rep.neutralized.max,
            rep.as_shipped.mean
        ));
    }
    json.push_str("  ],\n  \"tone\": [\n");

    let wedge_frames: Vec<&Frame> = m.frames.iter().filter(|f| f.role == "wedge").collect();
    let mut csv = String::from("frame,step,ev,lstar\n");
    for (idx, f) in wedge_frames.iter().enumerate() {
        let corners = f
            .corners
            .ok_or_else(|| format!("wedge frame {} missing corners", f.file))?;
        let neg = decode(&m.dir, &f.file);
        let rep = score_tone(
            &neg,
            base,
            &corners,
            f.n_steps.unwrap_or(10),
            f.ev_per_step.unwrap_or(1.0),
            f.mid_step.unwrap_or(0),
            f.drop_last.unwrap_or(0),
        );

        // Overlay for human verification (wedge: n_steps × 1 grid).
        let positive = invert_image(
            &neg,
            &InversionParams {
                base,
                ..Default::default()
            },
            Mode::D,
        );
        let spec = GridSpec {
            cols: f.n_steps.unwrap_or(10),
            rows: 1,
            inset: 0.5,
        };
        let ov = sampling_overlay(&positive, &corners, &spec, 1400);
        let ov_path = format!("{out_dir}/overlay_{}.png", sanitize(&f.file));
        ov.save(&ov_path)
            .map_err(|e| format!("save {ov_path}: {e}"))?;

        for (i, (e, l)) in rep.ev.iter().zip(rep.lstar.iter()).enumerate() {
            csv.push_str(&format!("{},{},{},{:.3}\n", f.file, i, e, l));
        }
        json.push_str(&format!(
            "    {{ \"file\": {:?}, \"mid_gray_l\": {:.2}, \"shadow_latitude_ev\": {:.2}, \"highlight_latitude_ev\": {:.2}, \"mid_slope\": {:.2}, \"monotonic\": {}, \"flags\": {} }}{}\n",
            f.file, rep.mid_gray_l, rep.shadow_latitude_ev, rep.highlight_latitude_ev, rep.mid_slope, rep.monotonic,
            json_str_array(&f.flags),
            if idx + 1 < wedge_frames.len() { "," } else { "" }
        ));
        summary.push(format!(
            "  {}: mid-gray L* {:.1}, shadow {:.1} EV, highlight {:.1} EV, slope {:.1}, monotonic {}",
            f.file, rep.mid_gray_l, rep.shadow_latitude_ev, rep.highlight_latitude_ev, rep.mid_slope, rep.monotonic
        ));
    }
    json.push_str("  ]\n}\n");

    std::fs::write(format!("{out_dir}/metrics.json"), json)
        .map_err(|e| format!("write metrics: {e}"))?;
    std::fs::write(format!("{out_dir}/tone_curve.csv"), csv)
        .map_err(|e| format!("write csv: {e}"))?;

    eprintln!("=== film-bench: {} ===", m.roll);
    for line in summary {
        eprintln!("{line}");
    }
    eprintln!("base = {base:?}");
    eprintln!("outputs in {out_dir}/ (metrics.json, tone_curve.csv, overlay_*.png)");
    Ok(())
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn json_str_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("{s:?}")).collect();
    format!("[{}]", inner.join(", "))
}
