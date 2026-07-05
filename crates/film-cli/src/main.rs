use anyhow::{Context, Result};
use clap::Parser;
use film_core::calibrate::{dmax_from_white_point, sample_base, sample_dmax, Rect};
use film_core::decode::decode_tiff;
use film_core::engine::{invert_image, InversionParams, Mode};
use film_core::export::write_tiff16;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "film-cli",
    about = "Invert a color negative scan (Kodak Cineon)"
)]
struct Cli {
    /// Input TIFF / linear DNG
    input: PathBuf,
    /// Output 16-bit TIFF
    #[arg(short, long)]
    output: PathBuf,
    /// Optional base-sample rect: x,y,w,h (defaults to whole image)
    #[arg(long, value_delimiter = ',')]
    base_rect: Option<Vec<usize>>,
    /// Optional measured white-point rect (exposed leader): x,y,w,h. When set,
    /// D_max is anchored to this leader instead of the scene-percentile estimate.
    #[arg(long, value_delimiter = ',')]
    white_rect: Option<Vec<usize>>,
    /// Print exposure in EV stops (→ linear print exposure = 2^exposure).
    #[arg(long, default_value = "1.0")]
    exposure: f32,
    /// Decode the input and report whether it carries an infrared plane, then exit.
    /// (Still requires a dummy `-o`, e.g. `-o /dev/null`.)
    #[arg(long)]
    check_ir: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ext = cli
        .input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let img = match ext.as_str() {
        "tif" | "tiff" => decode_tiff(&cli.input),
        "jpg" | "jpeg" | "png" => film_core::decode::decode_ldr(&cli.input),
        _ => film_core::decode::decode_raw(&cli.input, false),
    }
    .with_context(|| format!("decoding {:?}", cli.input))?;

    if cli.check_ir {
        match &img.ir {
            Some(ir) => println!(
                "{:?}: {}x{} RGB+IR (4-channel); ir samples = {}",
                cli.input,
                img.width,
                img.height,
                ir.len()
            ),
            None => println!(
                "{:?}: {}x{} RGB only — no infrared plane",
                cli.input, img.width, img.height
            ),
        }
        return Ok(());
    }

    let rect = cli.base_rect.as_ref().and_then(|v| {
        if v.len() == 4 {
            Some(Rect {
                x: v[0],
                y: v[1],
                w: v[2],
                h: v[3],
            })
        } else {
            None
        }
    });
    let base = sample_base(&img, rect);
    eprintln!("film base (orange mask) = {base:?}");

    // Parse an optional white-rect the same way as base_rect.
    let white_rect = cli.white_rect.as_ref().and_then(|v| {
        if v.len() == 4 {
            Some(Rect {
                x: v[0],
                y: v[1],
                w: v[2],
                h: v[3],
            })
        } else {
            None
        }
    });
    let d_max = match white_rect {
        Some(_) => dmax_from_white_point(&img, base, white_rect),
        None => sample_dmax(&img, base, None),
    };

    // One engine: Kodak Cineon (Mode D). The exposure slider drives print exposure;
    // d_max/paper_* come from InversionParams::Default.
    let params = InversionParams {
        base,
        print_exposure: 2f32.powf(cli.exposure),
        d_max,
        ..Default::default()
    };
    let out = invert_image(&img, &params, Mode::D);
    write_tiff16(&out, &cli.output).context("writing output")?;
    eprintln!("wrote {:?} (Cineon)", cli.output);
    Ok(())
}
