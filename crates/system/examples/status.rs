//! What this machine tells the shell now: the battery, the network, how
//! loud the sound is, what was copied lately, and how many applications
//! are installed. For trying the system's halves of `gevrek_system`:
//!
//!     cargo run -p gevrek-system --example status

fn main() {
    use gevrek_system::{apps, sound, status};
    println!("battery: {:?}", status::battery());
    println!("network: {:?}", status::network());
    println!("speaker: {:?}", sound::level(sound::Device::Speaker));
    println!("microphone: {:?}", sound::level(sound::Device::Microphone));
    println!("wifi: {:?}", gevrek_system::wifi::read());
    println!("bluetooth: {:?}", gevrek_system::bluetooth::read());
    println!("brightness: {:?}", gevrek_system::brightness::read().map(|now| now.percent()));
    // (By nothing: the call is tried, the display stays as it is.)
    println!("brightness changed by 0: {:?}", gevrek_system::brightness::change(0).map(|now| now.percent()));
    let installed = apps::installed();
    println!(
        "applications: {} ({})",
        installed.len(),
        installed.iter().take(5).map(|app| app.name.as_str()).collect::<Vec<_>>().join(", ")
    );
    if let Some(keeper) = gevrek_system::clipboard::start() {
        std::thread::sleep(std::time::Duration::from_millis(800));
        println!("copied lately: {} text(s)", keeper.copied().len());
    } else {
        println!("copied lately: not kept on this system");
    }
}
