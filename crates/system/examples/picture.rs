//! The picture of a window, as the window switcher asks for it, saved as
//! a PPM file: `cargo run -p gevrek-system --example picture -- <window> [side] [file]`.
fn main() {
    let handle: isize = std::env::args().nth(1).and_then(|text| text.parse().ok()).expect("a window's number");
    let side: u32 = std::env::args().nth(2).and_then(|text| text.parse().ok()).unwrap_or(256);
    let Some(picture) = gevrek_system::pictures::of_window(handle, side) else {
        println!("no picture");
        return;
    };
    println!("{}x{}", picture.width, picture.height);
    if let Some(file) = std::env::args().nth(3) {
        let mut bytes = format!("P6\n{} {}\n255\n", picture.width, picture.height).into_bytes();
        for pixel in picture.rgba.chunks(4) {
            bytes.extend_from_slice(&pixel[..3]);
        }
        std::fs::write(&file, bytes).expect("written");
    }
}
