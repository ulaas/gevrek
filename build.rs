//! Embeds the per-monitor-v2 DPI manifest into the `gevrek-wm` binary, and
//! gives the programs gevrek's icon on Windows (the "nightly" one, on the
//! crust-orange ground, in a build that is no release).

fn main() {
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/gevrek-wm.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg-bin=gevrek-wm=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=gevrek-wm=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }

    let brand = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/brand");
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let icon = brand.join("app-icon/windows").join(if release {
        "gevrek.ico"
    } else {
        "gevrek-nightly.ico"
    });
    println!("cargo:rerun-if-changed={}", icon.display());
    if target_os == "windows" {
        let script =
            std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icon.rc");
        // (The resource compiler reads backslashes as marks.)
        let path = icon.display().to_string().replace('\\', "/");
        std::fs::write(&script, format!("1 ICON \"{path}\"\n")).expect("the resource script");
        let _ = embed_resource::compile(&script, embed_resource::NONE);
    }
}
