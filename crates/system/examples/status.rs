//! What a panel would show of this machine: `cargo run -p gevrek-system
//! --example status`. The battery (or that there is none) and the network.

fn main() {
    match gevrek_system::status::battery() {
        Some(battery) => println!(
            "battery: {}%{}",
            battery.percent,
            if battery.charging { ", on the mains" } else { "" }
        ),
        None => println!("battery: none"),
    }
    match gevrek_system::status::network() {
        Some(network) if network.connected => println!(
            "network: {} \"{}\"",
            if network.wireless { "by radio" } else { "by cable" },
            network.name
        ),
        Some(_) => println!("network: not connected"),
        None => println!("network: nothing to connect with"),
    }
}
