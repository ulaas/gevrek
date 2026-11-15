//! Draws gevrek's mark (`assets/brand/logo/mark/*.svg`) as PNG files, for
//! the places that show a picture and read no SVG (the studio's top bar,
//! the panel's launcher button; `gevrek_ui::brand`).
//!
//! `cargo run -p gevrek-system --example brand_marks` writes them to
//! `assets/brand/logo/mark/png`. Run again when the marks change; what it
//! writes is kept in the repository.

fn main() {
    let brand = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brand");
    let from = brand.join("logo/mark");
    let to = from.join("png");
    std::fs::create_dir_all(&to).expect("the folder for the pictures");
    // The mark of five panes from 48 pixels up, the one of three below
    // (the brand's rule); each for a dark and for a light ground.
    for (name, sizes) in [
        ("gevrek-mark-small", [32u32, 64]),
        ("gevrek-mark", [128, 256]),
    ] {
        for ground in ["on-dark", "on-light"] {
            for size in sizes {
                let file = from.join(format!("{name}-{ground}.svg"));
                let Some(picture) = gevrek_system::icons::load(&file, size) else {
                    eprintln!("{} could not be read", file.display());
                    std::process::exit(1);
                };
                let out = to.join(format!("{name}-{ground}-{size}.png"));
                image::RgbaImage::from_raw(picture.width, picture.height, picture.pixels)
                    .expect("as many pixels as the size says")
                    .save(&out)
                    .expect("the picture written");
                println!("{}", out.display());
            }
        }
    }
}
