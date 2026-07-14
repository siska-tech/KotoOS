use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let rp2040 = env::var_os("CARGO_FEATURE_MCU_RP2040").is_some();
    let rp235xa = env::var_os("CARGO_FEATURE_MCU_RP235XA").is_some();
    let picocalc_pico = env::var_os("CARGO_FEATURE_BOARD_PICOCALC_PICO").is_some();
    let picocalc_pico2w = env::var_os("CARGO_FEATURE_BOARD_PICOCALC_PICO2W").is_some();
    let memory = match (rp2040, rp235xa) {
        (true, false) => include_bytes!("memory.x").as_slice(),
        (false, true) => include_bytes!("memory-rp235xa.x").as_slice(),
        // lib.rs emits the user-facing compile_error. Keep the build script
        // deterministic so Cargo can reach that diagnostic.
        _ => include_bytes!("memory.x").as_slice(),
    };
    fs::write(out.join("memory.x"), memory).expect("write selected MCU memory.x");

    if rp235xa && !rp2040 {
        fs::write(out.join("link-rp235x.x"), include_bytes!("link-rp235x.x"))
            .expect("write RP235x linker supplement");
    }

    let (board_id, mcu_id) = match (picocalc_pico, picocalc_pico2w) {
        (true, false) => ("picocalc-pico-rp2040", "rp2040"),
        (false, true) => ("picocalc-pico2w-rp2350a", "rp2350a"),
        _ => ("invalid-board-selection", "invalid-board-selection"),
    };
    println!("cargo:rustc-env=KOTO_BOARD_ID={board_id}");
    println!("cargo:rustc-env=KOTO_MCU_ID={mcu_id}");

    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=memory-rp235xa.x");
    println!("cargo:rerun-if-changed=link-rp235x.x");
}
