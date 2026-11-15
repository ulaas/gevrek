//! How loud the default devices are, as `gevrek_system::sound` reads them.
//!
//! `cargo run -p gevrek-system --example sound` prints the two levels and
//! changes nothing.
//!
//! On Windows, `-- watch 20` then listens for twenty seconds and prints
//! every change the system tells of (turn the volume by hand meanwhile).
//! `-- step` is for seeing that a change is told of without touching the
//! keys: it makes what is heard one in a hundred quieter and puts it back
//! exactly as it was half a second later, and prints what the system told.
//! It does nothing when the device is muted or nearly silent, and never
//! makes anything louder than it was.

use gevrek_system::sound::{self, Device};

fn main() {
    for device in [Device::Speaker, Device::Microphone] {
        match sound::level(device) {
            Some(level) => println!(
                "{device:?}: {} %{}",
                level.percent,
                if level.muted { ", muted" } else { "" }
            ),
            None => println!("{device:?}: no device answers"),
        }
    }
    for output in sound::outputs() {
        println!(
            "to play on: {} {}{}",
            output.id,
            output.name,
            if output.chosen { "  (in use)" } else { "" }
        );
    }
    // On Linux, what the mixer shows, and (with a word) a change: for a
    // sound server of a check's own (`PULSE_SERVER`), not the machine's.
    #[cfg(target_os = "linux")]
    {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let number = |index: usize| {
            arguments
                .get(index)
                .and_then(|text| text.parse::<i64>().ok())
        };
        let said = |level: Option<sound::Level>| match level {
            Some(level) => println!(
                "now: {} %{}",
                level.percent,
                if level.muted { ", muted" } else { "" }
            ),
            None => println!("now: nothing answers"),
        };
        match (arguments.first().map(String::as_str), number(1), number(2)) {
            (Some("change"), Some(by), _) => said(sound::change(Device::Speaker, by as i32)),
            (Some("mute"), ..) => said(sound::toggle_mute(Device::Speaker)),
            (Some("choose"), Some(id), _) => {
                println!("chosen: {}", sound::choose_output(id as u32))
            }
            (Some("stream"), Some(id), Some(percent)) => {
                println!("set: {}", sound::set_stream(id as u32, percent as u32))
            }
            (Some("stream-mute"), Some(id), _) => {
                println!("muted: {}", sound::toggle_stream_mute(id as u32))
            }
            _ => {}
        }
        let mixer = sound::mixer();
        for input in &mixer.inputs {
            println!(
                "to record from: {} {}{}",
                input.id,
                input.name,
                if input.chosen { "  (in use)" } else { "" }
            );
        }
        for stream in &mixer.streams {
            println!(
                "plays: {} {} at {} %{}",
                stream.id,
                stream.name,
                stream.level.percent,
                if stream.level.muted { ", muted" } else { "" }
            );
        }
    }
    #[cfg(windows)]
    {
        let mut arguments = std::env::args().skip(1);
        let what = arguments.next();
        let listen = || {
            sound::watch(|level| {
                println!(
                    "changed: {} %{}",
                    level.percent,
                    if level.muted { ", muted" } else { "" }
                );
            })
        };
        let wait = |milliseconds| {
            std::thread::sleep(std::time::Duration::from_millis(milliseconds));
        };
        match what.as_deref() {
            Some("watch") => {
                let seconds: u64 = arguments
                    .next()
                    .and_then(|text| text.parse().ok())
                    .unwrap_or(20);
                println!("listening={} for {seconds} s", listen());
                wait(seconds * 1000);
            }
            // The device in use is chosen again: nothing changes for the
            // ear, and the system is seen to take the choice.
            Some("rechoose") => match sound::outputs().into_iter().find(|output| output.chosen) {
                Some(output) => println!(
                    "chose {} \"{}\" again: taken={}",
                    output.id,
                    output.name,
                    sound::choose_output(output.id)
                ),
                None => println!("no device is in use: nothing done"),
            },
            Some("step") => {
                let (Some(was), Some(level)) = (
                    sound::measure(Device::Speaker),
                    sound::level(Device::Speaker),
                ) else {
                    println!("no device answers: nothing done");
                    return;
                };
                if level.muted || was < 0.05 {
                    println!("muted or nearly silent: nothing done");
                    return;
                }
                println!("listening={}", listen());
                wait(500);
                let lower = sound::set(Device::Speaker, was - 0.01);
                wait(500);
                let back = sound::set(Device::Speaker, was);
                wait(500);
                let now = sound::measure(Device::Speaker);
                println!(
                    "one step down: {lower:?}; put back: {back:?}; exactly as it was={}",
                    now == Some(was)
                );
            }
            _ => {}
        }
    }
}
