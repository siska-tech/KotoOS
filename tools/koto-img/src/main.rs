//! koto-img CLI (KOTO-0187 / KOTO-0196): PNG <-> `.kspr` sprite sheets and
//! `.kicon` launcher icons.

use std::process::ExitCode;

const USAGE: &str = "usage: koto-img png2kspr  IN.png   OUT.kspr\n\
                     \x20      koto-img kspr2png  IN.kspr  OUT.png\n\
                     \x20      koto-img kim2png   IN.kim   OUT.png\n\
                     \x20      koto-img png2kicon IN.png   OUT.kicon  (40x40 mask)\n\
                     \x20      koto-img kicon2png IN.kicon OUT.png";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, input, output) = match args.as_slice() {
        [mode, input, output] => (mode.as_str(), input, output),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    match run(mode, input, output) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("koto-img: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(mode: &str, input: &str, output: &str) -> Result<String, String> {
    match mode {
        "png2kspr" => {
            let bytes = read(input)?;
            let (w, h, rgba) = koto_img::decode_png_rgba(&bytes)?;
            let kspr = koto_img::rgba_to_kspr(w, h, &rgba)?;
            write(output, kspr.as_bytes())?;
            let tiles = (w as usize / koto_img::TILE) * (h as usize / koto_img::TILE);
            Ok(format!(
                "png2kspr: {input} ({w}x{h}) -> {output} ({tiles} tiles)"
            ))
        }
        "kspr2png" => {
            let text = read_text(input)?;
            let kim = koto_img::compile_kspr(&text).map_err(|e| format!("{input}: {e}"))?;
            let (w, h, rgb) = koto_img::kim_to_rgb8(&kim)?;
            let png = koto_img::encode_png_rgb(w as u32, h as u32, &rgb)?;
            write(output, &png)?;
            Ok(format!("kspr2png: {input} -> {output} ({w}x{h})"))
        }
        "kim2png" => {
            let bytes = read(input)?;
            let (w, h, rgb) = koto_img::kim_to_rgb8(&bytes).map_err(|e| format!("{input}: {e}"))?;
            let png = koto_img::encode_png_rgb(w as u32, h as u32, &rgb)?;
            write(output, &png)?;
            Ok(format!("kim2png: {input} -> {output} ({w}x{h})"))
        }
        "png2kicon" => {
            let bytes = read(input)?;
            let (w, h, rgba) = koto_img::decode_png_rgba(&bytes)?;
            let kicon = koto_img::rgba_to_kicon(w, h, &rgba)?;
            write(output, kicon.as_bytes())?;
            Ok(format!("png2kicon: {input} ({w}x{h}) -> {output}"))
        }
        "kicon2png" => {
            let text = read_text(input)?;
            let grid = koto_img::parse_kicon(&text).map_err(|e| format!("{input}: {e}"))?;
            let (w, h, rgb) = koto_img::kicon_to_rgb(&grid);
            let png = koto_img::encode_png_rgb(w, h, &rgb)?;
            write(output, &png)?;
            Ok(format!("kicon2png: {input} -> {output} ({w}x{h})"))
        }
        other => Err(format!("unknown mode {other:?}\n{USAGE}")),
    }
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn read_text(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn write(path: &str, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("{path}: {e}"))
}
