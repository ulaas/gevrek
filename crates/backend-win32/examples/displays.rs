//! The displays as Windows has them, and what a `desktop.outputs` would
//! change about them: tried with Windows' own test, never set.
//!
//! `cargo run -p gevrek-win32 --example displays` lists the displays.
//! With a file of entries (`[{"name": "DISPLAY2", "position": [-2560, 0]}]`)
//! it says what would be asked of Windows and whether Windows would take it.
//! Nothing on the screen changes.

fn main() {
    let displays = gevrek_win32::display_settings::displays();
    for display in &displays {
        println!(
            "{} ({}): {}x{} at {} Hz at ({}, {}), {:?}{}, {} modes",
            display.device,
            display.name,
            display.width,
            display.height,
            display.hertz,
            display.x,
            display.y,
            display.turned,
            if display.primary { ", the main one" } else { "" },
            display.modes.len()
        );
    }
    let Some(file) = std::env::args().nth(1) else {
        return;
    };
    let entries: Vec<gevrek_model::outputs::OutputProfile> = match std::fs::read_to_string(&file)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str(&text).map_err(|error| error.to_string()))
    {
        Ok(entries) => entries,
        Err(error) => {
            println!("{file}: {error}");
            std::process::exit(2);
        }
    };
    match gevrek_win32::display_settings::plan(&entries, &displays) {
        Err(why) => println!("refused before Windows was asked: {why}"),
        Ok(wanted) if wanted.is_empty() => println!("nothing to change"),
        Ok(wanted) => {
            for display in &wanted {
                println!(
                    "would ask: {} {}x{} at {} Hz at ({}, {}), {:?}{}",
                    display.device,
                    display.width,
                    display.height,
                    display.hertz,
                    display.x,
                    display.y,
                    display.turned,
                    if display.primary { ", the main one" } else { "" }
                );
            }
            match gevrek_win32::display_settings::test(&wanted) {
                Ok(()) => println!("Windows would take it"),
                Err(why) => println!("Windows would not take it: {why}"),
            }
        }
    }
}
