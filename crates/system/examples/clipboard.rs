//! Keep what is copied for a few seconds, and say what was kept:
//!
//!     cargo run -p gevrek-system --example clipboard -- 5
fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|text| text.parse().ok()).unwrap_or(5);
    let Some(keeper) = gevrek_system::clipboard::start() else {
        println!("what is copied is not kept on this system");
        return;
    };
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    println!("{:?}", keeper.copied());
}
