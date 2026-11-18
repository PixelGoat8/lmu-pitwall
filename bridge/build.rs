use std::{env, fs, path::PathBuf};

/// Embeds the application icon in the Windows executable, with the Windows
/// SDK's resource compiler (found by `embed-resource`). The team and the CI
/// build with the MSVC toolchain on Windows; other targets get no icon.
fn main() {
    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let icon_path = manifest.join("assets").join("icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    if !icon_path.exists() {
        println!("cargo:warning=assets/icon.ico not found: the executable gets no icon");
        return;
    }

    // A one-line resource script with the icon's absolute path, escaped for rc.
    let rc_path = out_dir.join("icon.rc");
    let icon = icon_path.to_str().expect("icon path is valid UTF-8").replace('\\', "\\\\");
    fs::write(&rc_path, format!("IDI_ICON1 ICON \"{icon}\"\n")).expect("write icon.rc");

    // Reports its own problems as cargo warnings; the executable works without the icon.
    embed_resource::compile(&rc_path, embed_resource::NONE);
}
