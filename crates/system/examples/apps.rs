//! List the installed applications, or those that answer a word:
//! `cargo run -p gevrek-system --example apps -- fire`.
//!
//! With `--icon <word> <file.png>`, write the icon of the first that
//! answers the word, as a launcher gets it.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let began = std::time::Instant::now();
    let apps = gevrek_system::apps::installed();
    let took = began.elapsed();
    if arguments.first().map(String::as_str) == Some("--icon") {
        let (Some(typed), Some(file)) = (arguments.get(1), arguments.get(2)) else {
            eprintln!("--icon <word> <file.png>");
            std::process::exit(2);
        };
        let Some(app) = gevrek_system::apps::search(&apps, typed).into_iter().next() else {
            eprintln!("no application answers {typed}");
            std::process::exit(1);
        };
        let began = std::time::Instant::now();
        let Some(picture) = gevrek_system::icons::picture(&app.icon, 64) else {
            eprintln!("{} has no icon ({})", app.name, app.icon);
            std::process::exit(1);
        };
        println!(
            "{}: {} by {} pixels in {:?}",
            app.name,
            picture.width,
            picture.height,
            began.elapsed()
        );
        if let Err(error) = image::save_buffer(
            file,
            &picture.pixels,
            picture.width,
            picture.height,
            image::ExtendedColorType::Rgba8,
        ) {
            eprintln!("{file}: {error}");
            std::process::exit(1);
        }
        return;
    }
    let typed = arguments.first().cloned().unwrap_or_default();
    let found = gevrek_system::apps::search(&apps, &typed);
    for app in found.iter().take(15) {
        println!("{:<34} {:<26} {}", app.name, app.id, app.command.join(" "));
    }
    println!(
        "{} of {} applications (read in {took:?})",
        found.len(),
        apps.len()
    );
    // Where they come from on Windows, and how much was read for them.
    #[cfg(windows)]
    {
        let packaged = apps
            .iter()
            .filter(|app| app.entry.as_os_str().is_empty())
            .count();
        println!(
            "{} from {} shortcuts of the Start menu, {packaged} packaged of {} entries in the folder of applications",
            apps.len() - packaged,
            gevrek_system::start_menu::read_shortcuts().len(),
            gevrek_system::start_menu::read_packaged().len(),
        );
    }
}
