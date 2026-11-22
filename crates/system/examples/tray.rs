//! The tray's icons as the host finds them, for a few seconds:
//! `cargo run -p gevrek-system --example tray -- 5`.
#[cfg(any(windows, target_os = "macos"))]
fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|text| text.parse().ok()).unwrap_or(5);
    let host = gevrek_system::tray_host::Host::start(|icons| {
        println!("{} icon(s):", icons.len());
        for icon in icons {
            let picture = icon.picture.as_ref().map(|picture| format!("{}x{}", picture.width, picture.height));
            println!("  {:<10} {:<44} {:?} picture {:?}", icon.key, icon.name, icon.tip, picture);
            // GEVREK_TRAY_SAVE=folder: each picture as a PPM file there.
            if let (Ok(folder), Some(picture)) = (std::env::var("GEVREK_TRAY_SAVE"), &icon.picture) {
                let mut bytes = format!("P6\n{} {}\n255\n", picture.width, picture.height).into_bytes();
                // (Over a checkered ground, so what is clear shows.)
                for (index, pixel) in picture.pixels.as_chunks::<4>().0.iter().enumerate() {
                    let (x, y) = (index as u32 % picture.width, index as u32 / picture.width);
                    let under = if (x / 4 + y / 4) % 2 == 0 { 200u32 } else { 120 };
                    let alpha = u32::from(pixel[3]);
                    for colour in &pixel[..3] {
                        bytes.push(((u32::from(*colour) * alpha + under * (255 - alpha)) / 255) as u8);
                    }
                }
                let _ = std::fs::write(format!("{folder}/{}.ppm", icon.name.replace(':', "-")), bytes);
            }
        }
    });
    println!("started: {}", host.is_some());
    // GEVREK_TRAY_PRESS=key: that icon pressed after a moment.
    if let (Ok(key), Some(host)) = (std::env::var("GEVREK_TRAY_PRESS"), &host) {
        std::thread::sleep(std::time::Duration::from_secs(1));
        host.press(&key, gevrek_system::tray_host::Press::Main);
        println!("pressed {key}");
    }
    std::thread::sleep(std::time::Duration::from_secs(seconds));
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {}
