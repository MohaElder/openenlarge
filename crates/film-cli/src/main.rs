use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use film_core::calibrate::{sample_base, Rect};
use film_core::decode::decode_tiff;
use film_core::engine::{invert_image, InversionParams, Mode};
use film_core::export::write_tiff16;
use film_core::spectral::Stock;
use std::path::PathBuf;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum CliMode {
    B,
    C,
    Naive,
}

impl From<CliMode> for Mode {
    fn from(m: CliMode) -> Self {
        match m {
            CliMode::B => Mode::B,
            CliMode::C => Mode::C,
            CliMode::Naive => Mode::Naive,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum CliStock {
    None,
    Portra400,
    FujiC200,
}

impl CliStock {
    fn to_stock(self) -> Option<Stock> {
        match self {
            CliStock::None => None,
            CliStock::Portra400 => Some(Stock::Portra400),
            CliStock::FujiC200 => Some(Stock::FujiC200),
        }
    }
}

#[derive(Parser)]
#[command(name = "film-cli", about = "Invert a color negative scan")]
struct Cli {
    /// Input TIFF / linear DNG
    input: PathBuf,
    /// Output 16-bit TIFF
    #[arg(short, long)]
    output: PathBuf,
    /// Inversion mode
    #[arg(long, value_enum, default_value = "b")]
    mode: CliMode,
    /// Optional base-sample rect: x,y,w,h (defaults to whole image)
    #[arg(long, value_delimiter = ',')]
    base_rect: Option<Vec<usize>>,
    #[arg(long, default_value = "1.0")]
    exposure: f32,
    #[arg(long, default_value = "0.0")]
    black: f32,
    #[arg(long, default_value = "0.4545")]
    gamma: f32,
    /// Decode the input and report whether it carries an infrared plane, then exit.
    /// (Still requires a dummy `-o`, e.g. `-o /dev/null`.)
    #[arg(long)]
    check_ir: bool,
    /// Emit B, C, and naive outputs side by side (writes <output stem>_{b,c,naive}.tiff)
    #[arg(long)]
    compare: bool,
    /// Film stock for Mode B density unmixing (fits M_post). `none` = identity.
    #[arg(long, value_enum, default_value = "none")]
    stock: CliStock,
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
        _ => film_core::decode::decode_raw(&cli.input),
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

    let params = InversionParams {
        base,
        exposure: cli.exposure,
        black: cli.black,
        gamma: cli.gamma,
        ..Default::default()
    };

    let b_params = match cli.stock.to_stock() {
        Some(s) => {
            eprintln!("using fitted M_post for stock {:?}", cli.stock);
            film_core::engine::params_for_stock(s, base, cli.exposure, cli.black, cli.gamma)
        }
        None => params.clone(),
    };

    if cli.compare {
        let stem = cli
            .output
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out")
            .to_string();
        let dir = cli
            .output
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        for (mode, suffix) in [(Mode::B, "b"), (Mode::C, "c"), (Mode::Naive, "naive")] {
            let p = if mode == Mode::B { &b_params } else { &params };
            let out = invert_image(&img, p, mode);
            let path = dir.join(format!("{stem}_{suffix}.tiff"));
            write_tiff16(&out, &path).context("writing compare output")?;
            eprintln!("wrote {path:?}");
        }
        return Ok(());
    }

    let mode: Mode = cli.mode.into();
    let chosen = if mode == Mode::B { &b_params } else { &params };
    let out = invert_image(&img, chosen, mode);
    write_tiff16(&out, &cli.output).context("writing output")?;
    eprintln!("wrote {:?} ({:?} mode)", cli.output, cli.mode);
    Ok(())
}
