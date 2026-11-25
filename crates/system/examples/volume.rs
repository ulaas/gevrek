//! Say how loud what is heard is each time that changes, for a few
//! seconds: `cargo run -p gevrek-system --example volume -- 5`.
#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("changes of the volume are told on Windows and macOS");
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(5);
    println!(
        "now: {:?}",
        gevrek_system::sound::level(gevrek_system::sound::Device::Speaker)
    );
    let watched = gevrek_system::sound::watch(|level| println!("changed: {level:?}"));
    println!("watched: {watched}");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
}
