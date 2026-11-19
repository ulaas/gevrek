//! What the zones' backend finds on this desktop: the items with their
//! facts, an icon asked for and fetched, the displays, and where
//! Explorer has its own icons.
//!
//! It only reads. Nothing on the desktop, in any folder or of Explorer's
//! is changed, unless it is started with `--really-hide`: then Explorer's
//! own icons are written down, put away, and brought back when Enter is
//! pressed. That is for trying the journal by hand, on a desktop that is
//! yours.
//!
//! ```text
//! cargo run -p gevrek-system --example zones_desktop
//! cargo run -p gevrek-system --example zones_desktop -- --really-hide
//! ```
//!
//! On Linux it lists the desktop folder, or the folder given: each item
//! with its facts, the program that opens it, the file of its icon and
//! the file of its small picture where it has one. `--thumbnails` makes
//! the small pictures that are missing (into the session's own store);
//! `--watch` stays and prints what changes until it is stopped.
//!
//! ```text
//! cargo run -p gevrek-system --example zones_desktop -- [folder] [--thumbnails] [--watch]
//! ```

#[cfg(windows)]
fn main() {
    windows_side::run();
}

#[cfg(not(windows))]
fn main() {
    linux_side::run();
}

#[cfg(windows)]
mod windows_side {
    use gevrek_system::zones::displays;
    use gevrek_system::zones::explorer::{Explorer, ShellDesktop};
    use gevrek_system::zones::places::Places;
    use gevrek_system::zones::{launch, SystemBackend};
    use gevrek_zones::backend::{BackendEvent, IconSize, Location, ZonesBackend};
    use gevrek_zones::config::SmartSource;
    use std::sync::mpsc;
    use std::time::Duration;

    pub fn run() {
        let really_hide = std::env::args().any(|argument| argument == "--really-hide");
        // The journal of this trial is kept apart from gevrek's own.
        let state = std::env::temp_dir().join("gevrek-zones-desktop-example");
        let explorer = if really_hide { ShellDesktop::new() } else { ShellDesktop::read_only() };
        let (woken, wakes) = mpsc::channel();
        let woken = std::sync::Mutex::new(woken);
        // (With the person's own Downloads and Recent folders, which are
        // only read.)
        let mut backend = SystemBackend::with(Places::of_session(&state), Box::new(explorer), move || {
            if let Ok(woken) = woken.lock() {
                let _ = woken.send(());
            }
        });

        println!("== The displays ==");
        for found in displays::from_system() {
            println!(
                "  {}  {}  pixels {:?}  work {:?}  scale {}{}",
                found.id,
                found.name,
                found.pixels,
                found.work,
                found.scale,
                if found.primary { "  (main)" } else { "" }
            );
        }
        for display in backend.displays() {
            println!("  as the core has it: {}  {:?}  scale {}", display.id, display.rect, display.scale);
        }
        for kept in backend.reserved_areas() {
            println!("  kept: {:?} {} of {}", kept.edge, kept.thickness, kept.display);
        }

        println!("\n== The desktop, as the shell lists it ==");
        let items = match backend.list(&Location::Desktop) {
            Ok(items) => items,
            Err(error) => {
                println!("  not listed: {error}");
                return;
            }
        };
        for item in &items {
            println!("  [{:?}] {}", item.kind, item.name);
            println!("      id      {}", item.id);
            let facts = &item.facts;
            let mut said = Vec::new();
            if let Some(extension) = &facts.extension {
                said.push(format!("ending {extension}"));
            }
            if let Some(mime) = &facts.mime {
                said.push(format!("type {mime}"));
            }
            if let Some(size) = facts.size {
                said.push(format!("{size} bytes"));
            }
            if let Some(modified) = facts.modified {
                said.push(format!("changed {modified}"));
            }
            if !said.is_empty() {
                println!("      facts   {}", said.join(", "));
            }
            if let Some(target) = &facts.shortcut_target {
                println!("      leads to {target}");
            }
        }
        println!("  {} items", items.len());

        println!("\n== Icons, asked for and fetched ==");
        let size = IconSize { size: 48, scale: 1.5 };
        let asked: Vec<_> = items
            .iter()
            .take(6)
            .map(|item| (backend.request_icon(&item.id, size), item.name.clone()))
            .collect();
        let mut answers = Vec::new();
        while answers.len() < asked.len() {
            // Woken by the backend when a picture is made; nothing is
            // looked at by the clock.
            if wakes.recv_timeout(Duration::from_secs(20)).is_err() {
                break;
            }
            for event in backend.drain_events() {
                if let BackendEvent::IconReady { request, icon } = event {
                    answers.push((request, icon));
                }
            }
        }
        for (request, name) in &asked {
            let icon = answers.iter().find(|(answered, _)| answered == request).and_then(|(_, icon)| *icon);
            match icon.and_then(|icon| backend.picture(icon).map(|picture| (icon, picture))) {
                Some((icon, picture)) => {
                    let solid = picture.pixels.as_chunks::<4>().0.iter().filter(|pixel| pixel[3] != 0).count();
                    println!(
                        "  {name}: handle {} -> {} x {} pixels, {solid} of them not clear",
                        icon.0, picture.width, picture.height
                    );
                }
                None => println!("  {name}: no picture"),
            }
        }

        if let Some(item) = items.iter().find(|item| item.path.is_some()) {
            println!("\n== The shell's menu for {} (not shown, only read) ==", item.name);
            match launch::menu_entries(std::slice::from_ref(&item.id)) {
                Ok(entries) => {
                    let texts: Vec<&str> = entries.iter().map(String::as_str).filter(|text| !text.is_empty()).collect();
                    println!("  {} entries: {}", texts.len(), texts.join(" | "));
                }
                Err(error) => println!("  not read: {error}"),
            }
        }

        println!("\n== What smart zones would show (only read) ==");
        let sources = [SmartSource::Recent, SmartSource::Downloads];
        let mut listed: Vec<(SmartSource, Vec<gevrek_zones::item::Item>)> = Vec::new();
        for source in &sources {
            if let Err(error) = backend.watch_source(source) {
                println!("  {source:?}: {error}");
                listed.push((source.clone(), Vec::new()));
            }
        }
        // Each list is read on a thread of its own and told when it is
        // there; woken by the backend, nothing is looked at by the clock.
        while listed.len() < sources.len() {
            if wakes.recv_timeout(Duration::from_secs(20)).is_err() {
                break;
            }
            for event in backend.drain_events() {
                if let BackendEvent::SourceListed { source, items } = event {
                    listed.retain(|(known, _)| known != &source);
                    listed.push((source, items));
                }
            }
        }
        for (source, items) in &listed {
            println!("  {source:?}: {} items, the first of them:", items.len());
            for item in items.iter().take(8) {
                let when = item.facts.accessed.or(item.facts.created).unwrap_or(0);
                println!("    {when:>10}  {}   [{}]", item.name, item.id);
            }
        }

        println!("\n== Explorer's own icons ==");
        let mut desktop = ShellDesktop::read_only();
        match desktop.read() {
            Ok(shown) => {
                println!(
                    "  icons shown: {}, auto-arrange: {}, snap to grid: {}, view {}, icon size {}",
                    shown.view.icons_shown,
                    shown.view.auto_arrange,
                    shown.view.snap_to_grid,
                    shown.view.view_mode,
                    shown.view.icon_size
                );
                for icon in &shown.icons {
                    println!("  ({:>5}, {:>5})  {}   [{}]", icon.x, icon.y, icon.name, icon.id);
                }
                println!("  {} icons", shown.icons.len());
                let listed = |id: &str| items.iter().any(|item| item.id.as_str().eq_ignore_ascii_case(id));
                let only_explorer: Vec<&str> =
                    shown.icons.iter().filter(|icon| !listed(&icon.id)).map(|icon| icon.name.as_str()).collect();
                let only_shell: Vec<&str> = items
                    .iter()
                    .filter(|item| !shown.icons.iter().any(|icon| icon.id.eq_ignore_ascii_case(item.id.as_str())))
                    .map(|item| item.name.as_str())
                    .collect();
                println!("  only Explorer shows: {only_explorer:?}");
                println!("  only the listing has: {only_shell:?}");
            }
            Err(error) => println!("  not read: {error}"),
        }
        println!("  a journal is open: {}", backend.system_icons_to_restore());

        if really_hide {
            println!("\n== Putting Explorer's icons away ==");
            if let Err(error) = backend.journal_system_icons() {
                println!("  not written down, nothing is changed: {error}");
                return;
            }
            println!("  written down under {}", state.display());
            match backend.hide_system_icons() {
                Ok(()) => println!("  put away. Press Enter to bring them back."),
                Err(error) => println!("  not put away: {error}. Press Enter to put back what was written down."),
            }
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            match backend.restore_system_icons() {
                Ok(()) => println!("  brought back."),
                Err(error) => println!(
                    "  NOT brought back: {error}. Run this again with --really-hide, or tick \"Show desktop icons\" in the desktop's menu."
                ),
            }
        }
    }
}

#[cfg(not(windows))]
mod linux_side {
    use gevrek_system::zones::{pixels, SystemBackend};
    use gevrek_zones::backend::{IconSize, Location, ZonesBackend};
    use std::path::Path;

    pub fn run() {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let flag = |name: &str| arguments.iter().any(|argument| argument == name);
        let folder = arguments.iter().find(|argument| !argument.starts_with("--"));
        let location = match folder {
            Some(path) => Location::Directory { path: path.clone() },
            None => Location::Desktop,
        };
        let (wake, woken) = std::sync::mpsc::channel();
        let mut backend = SystemBackend::new(move || {
            let _ = wake.send(());
        });
        println!("{}", backend.system().folder_of(&location).display());
        // Watching begins before the listing, as the zones' host does it.
        let watching = flag("--watch").then(|| backend.watch(&location));
        let items = match backend.list(&location) {
            Ok(items) => items,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        };
        let size = IconSize { size: 48, scale: 1.0 };
        for item in &items {
            let facts = &item.facts;
            println!("\n{}  ({:?})", item.name, item.kind);
            println!("  id         {}", item.id);
            let said = |value: Option<String>| value.unwrap_or_else(|| "-".to_string());
            println!("  type       {}", said(facts.mime.clone()));
            println!("  extension  {}", said(facts.extension.clone()));
            println!("  size       {}", said(facts.size.map(|size| size.to_string())));
            println!("  modified   {}", said(facts.modified.map(|time| time.to_string())));
            println!("  leads to   {}", said(facts.shortcut_target.clone()));
            let Some(path) = item.path.as_deref().map(Path::new) else {
                continue;
            };
            let system = backend.system();
            let Some(described) = system.describe(path) else {
                continue;
            };
            let program = system.default_program(path).map(|program| format!("{} ({})", program.name, program.id));
            println!("  opens with {}", said(program));
            let icon = system.icon_file(&described.icon_names, pixels(size));
            println!(
                "  icon       {}",
                icon.map_or_else(|| format!("none of {:?}", described.icon_names), |file| file.display().to_string())
            );
            if flag("--thumbnails") {
                let _ = system.thumbnail(path, pixels(size));
            }
            let small = system.thumbnail_file(path, pixels(size));
            println!("  thumbnail  {}", said(small.map(|file| file.display().to_string())));
        }
        println!("\n{} items", items.len());
        match watching {
            None => {}
            Some(Err(error)) => eprintln!("not watched: {error}"),
            Some(Ok(_)) => {
                println!("watching; stop with Ctrl+C");
                while woken.recv().is_ok() {
                    for event in backend.drain_events() {
                        println!("{event:?}");
                    }
                }
            }
        }
    }
}
