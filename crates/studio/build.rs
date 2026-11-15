//! Gives `gevrek-studio.exe` gevrek's icon on Windows (what the taskbar,
//! the Start menu and the file manager show for the program).
//!
//! A build that is no release has the "nightly" icon (the crust-orange
//! ground), so that it is told from an installed release in the taskbar.

fn main() {
    let brand = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brand");
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    let icon = brand.join("app-icon/windows").join(if release {
        "gevrek.ico"
    } else {
        "gevrek-nightly.ico"
    });
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|system| system == "windows") {
        let script = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icon.rc");
        // (The resource compiler reads backslashes as marks.)
        let path = icon.display().to_string().replace('\\', "/");
        std::fs::write(&script, format!("1 ICON \"{path}\"\n")).expect("the resource script");
        let _ = embed_resource::compile(&script, embed_resource::NONE);
    }
}
