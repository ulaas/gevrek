//! gevrek-lock: the lock screen.
//!
//! It asks the compositor to lock the session (the session-lock
//! protocol): from then on every display shows only this program's
//! surface and every key goes to it, and if this program dies the session
//! stays locked. The password is checked by the system (PAM), as the
//! user's own, on a thread of its own so the screen keeps drawing; it is
//! held only until it is checked and is not written anywhere.
//!
//! The window manager starts it for the shell action `lock`
//! (Super+Alt+L in the standard keys) and when the session has been idle.
//!
//! `gevrek-lock --greeter` is the login: the same screen with a field
//! for the name, in a gevrek that gevrek's display manager started with
//! nobody logged in. It checks nothing itself: the name and the password
//! go to the display manager (the socket named in `GEVREK_DM_SOCKET`),
//! which opens the session; this screen stays for the next login.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("gevrek-lock is for Linux: on Windows the system's own lock is used (Win+L).");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use gevrek_shellkit::layer::{Draw, Shell};
    use gevrek_shellkit::Flow;
    use gevrek_ui::theme::{self, icons};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    /// What the check of a password came to.
    enum Message {
        Verdict(Result<(), String>),
        /// The wallpaper has been read.
        Wallpaper(egui::ColorImage),
    }

    /// A colour written as `rrggbb`.
    fn accent_told(text: &str) -> Option<(u8, u8, u8)> {
        let text = text.trim().trim_start_matches('#');
        if text.len() != 6 {
            return None;
        }
        let value = u32::from_str_radix(text, 16).ok()?;
        Some(((value >> 16) as u8, (value >> 8) as u8, value as u8))
    }

    /// The largest side the wallpaper is kept at: it is drawn darkened
    /// behind the clock, and one picture serves every display.
    const WALLPAPER_SIDE: u32 = 2560;

    /// The wallpaper's file: what the compositor says is shown now
    /// (`GEVREK_LOCK_WALLPAPER`, empty: none), else what the
    /// configuration's file says.
    fn wallpaper_file(config: Option<&gevrek_model::Config>) -> Option<std::path::PathBuf> {
        match std::env::var("GEVREK_LOCK_WALLPAPER") {
            Ok(told) if told.is_empty() => None,
            Ok(told) => Some(told.into()),
            Err(_) => {
                let config = config?;
                config
                    .desktop
                    .session
                    .lock_shows_wallpaper
                    .then(|| config.desktop.wallpaper.of("").0)
                    .flatten()
            }
        }
    }

    /// The part of a picture of `picture` size that covers an area of
    /// `area` size when the picture keeps its shape: from where to where,
    /// in parts of the picture (what does not fit is cut off evenly).
    fn covering(picture: (f32, f32), area: (f32, f32)) -> ((f32, f32), (f32, f32)) {
        if picture.0 <= 0.0 || picture.1 <= 0.0 || area.0 <= 0.0 || area.1 <= 0.0 {
            return ((0.0, 0.0), (1.0, 1.0));
        }
        let (wide, high) = (picture.0 / area.0, picture.1 / area.1);
        // The side that is too long is shown in part.
        let shown = (
            (wide.min(high) / wide).min(1.0),
            (wide.min(high) / high).min(1.0),
        );
        (
            ((1.0 - shown.0) / 2.0, (1.0 - shown.1) / 2.0),
            ((1.0 + shown.0) / 2.0, (1.0 + shown.1) / 2.0),
        )
    }

    #[derive(PartialEq)]
    enum Status {
        Waiting,
        Checking,
        /// Refused, at this time.
        Wrong(Instant, String),
    }

    struct State {
        password: String,
        status: Status,
        user: String,
        settings: gevrek_ui::Settings,
        accent: egui::Color32,
        /// To hand a password to the thread that checks it.
        check: std::sync::mpsc::Sender<String>,
        unlocked: bool,
        /// The login, not the lock: the name is asked for too, and the
        /// screen stays when the password was right.
        greeter: bool,
        /// The wallpaper, once it has been read.
        wallpaper: Option<egui::ColorImage>,
        /// Since when the wallpaper is there: it comes in over a moment
        /// and does not jump into the picture.
        wallpaper_since: Option<Instant>,
        /// The accent is to be the wallpaper's own colour, once the
        /// wallpaper is read (`GEVREK_LOCK_ACCENT=wallpaper`).
        accent_of_wallpaper: bool,
        /// The keyboard's layouts, as they are named to the person
        /// (`US`, `TR`), in their order.
        layouts: Vec<String>,
        /// The login, until its first picture is drawn: where the
        /// display manager is told so (a boot splash it kept ends then).
        tell_shown: Option<std::path::PathBuf>,
    }

    /// The PAM service the password is checked with: gevrek's own when it
    /// is installed, else the one every login on this system goes through.
    pub fn service() -> &'static str {
        ["gevrek-lock", "system-auth", "login", "common-auth"]
            .into_iter()
            .find(|name| std::path::Path::new("/etc/pam.d").join(name).is_file())
            .unwrap_or("login")
    }

    /// Ask the system whether `password` is `user`'s.
    fn check(user: &str, password: &str) -> Result<(), String> {
        let mut client = pam::Client::with_password(service())
            .map_err(|error| format!("the system's check is not there: {error}"))?;
        client.conversation_mut().set_credentials(user, password);
        client
            .authenticate()
            .map_err(|_| "That is not the password.".to_string())
    }

    /// Where the greeter keeps the name that logged in last: with what
    /// its own user's programs remember (`XDG_STATE_HOME`, else
    /// `~/.local/state`).
    fn last_user_file() -> Option<std::path::PathBuf> {
        let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let state = set("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                set("HOME").map(|home| std::path::Path::new(&home).join(".local").join("state"))
            })?;
        Some(state.join("gevrek").join("greeter-last-user"))
    }

    /// Ask the display manager for something, and hear what it says.
    fn ask(socket: &std::path::Path, ask: &gevrek_dm::wire::Ask) -> Result<(), String> {
        use std::io::{BufRead, Write};
        let gone = |_| "The login service does not answer.".to_string();
        let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(gone)?;
        let mut line = serde_json::to_string(ask).map_err(|error| error.to_string())?;
        line.push('\n');
        let sent = stream.write_all(line.as_bytes());
        forget(&mut line);
        sent.map_err(gone)?;
        let mut heard = String::new();
        std::io::BufReader::new(stream)
            .read_line(&mut heard)
            .map_err(gone)?;
        let answer: gevrek_dm::wire::Answer = serde_json::from_str(&heard)
            .map_err(|_| "The login service does not answer.".to_string())?;
        if answer.ok {
            Ok(())
        } else {
            Err(answer.why)
        }
    }

    /// The keyboard's layouts as they are named to the person (`TR`):
    /// those of the session's configuration, else those the system (or,
    /// at the login, the display manager) names in `XKB_DEFAULT_LAYOUT`.
    fn layouts_known(config: Option<&gevrek_model::Config>) -> Vec<String> {
        let configured: Vec<String> = config
            .map(|config| {
                config
                    .desktop
                    .input
                    .keyboard
                    .layouts
                    .iter()
                    .map(|layout| layout.layout.clone())
                    .collect()
            })
            .unwrap_or_default();
        let said = if configured.is_empty() {
            std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default()
        } else {
            configured.join(",")
        };
        layouts_said(&said)
    }

    /// `tr,us` as `["TR", "US"]`.
    pub fn layouts_said(layouts: &str) -> Vec<String> {
        layouts
            .split(',')
            .map(str::trim)
            .filter(|layout| !layout.is_empty())
            .map(str::to_uppercase)
            .collect()
    }

    /// Overwrite a password before its memory is given back.
    fn forget(password: &mut String) {
        // SAFETY: zero bytes are valid UTF-8, and the length is unchanged.
        unsafe { password.as_bytes_mut() }.fill(0);
        password.clear();
    }

    pub fn run() -> anyhow::Result<()> {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
        let greeter = std::env::args().nth(1).as_deref() == Some("--greeter");
        let socket = std::env::var_os(gevrek_dm::SOCKET_VARIABLE).map(std::path::PathBuf::from);
        anyhow::ensure!(
            !greeter || socket.is_some(),
            "--greeter is for gevrek's display manager, which says where it listens ({})",
            gevrek_dm::SOCKET_VARIABLE
        );
        let user = if greeter {
            // Who logged in here last: the name is there already.
            last_user_file()
                .and_then(|file| std::fs::read_to_string(file).ok())
                .map(|name| name.trim().to_string())
                .filter(|name| gevrek_dm::wire::is_a_user_name(name))
                .unwrap_or_default()
        } else {
            std::env::var("USER")
                .or_else(|_| std::env::var("LOGNAME"))
                .unwrap_or_default()
        };
        anyhow::ensure!(
            greeter || !user.is_empty(),
            "who is logged in is not known (USER is not set): the screen is not locked"
        );
        let shown_socket = socket.clone().filter(|_| greeter);
        let mut shell: Shell<Message> = Shell::new()?;

        // The thread that checks: one password at a time. (The greeter
        // hands over the name and the password as two lines of one text.)
        let (to_check, passwords) = std::sync::mpsc::channel::<String>();
        let verdicts = shell.sender();
        let checked_user = user.clone();
        std::thread::Builder::new()
            .name("gevrek-lock-check".to_string())
            .spawn(move || {
                for mut password in passwords {
                    let verdict = match &socket {
                        Some(socket) if greeter => match password.split_once('\n') {
                            Some(("", "poweroff")) => {
                                ask(socket, &gevrek_dm::wire::Ask::Power { restart: false })
                            }
                            Some(("", "restart")) => {
                                ask(socket, &gevrek_dm::wire::Ask::Power { restart: true })
                            }
                            Some((name, word)) => {
                                let mut login = gevrek_dm::wire::Ask::Login {
                                    user: name.to_string(),
                                    password: word.to_string(),
                                };
                                let verdict = ask(socket, &login);
                                if verdict.is_ok() {
                                    if let Some(file) = last_user_file() {
                                        if let Some(folder) = file.parent() {
                                            let _ = std::fs::create_dir_all(folder);
                                        }
                                        let _ = std::fs::write(file, name);
                                    }
                                }
                                if let gevrek_dm::wire::Ask::Login { password, .. } = &mut login {
                                    forget(password);
                                }
                                verdict
                            }
                            None => Err("The check could not be started.".to_string()),
                        },
                        _ => check(&checked_user, &password),
                    };
                    forget(&mut password);
                    if !verdicts.send(Message::Verdict(verdict)) {
                        return;
                    }
                }
            })?;

        let appearance = gevrek_model::paths::config_file()
            .parent()
            .map(|folder| folder.join("studio-appearance"))
            .unwrap_or_default();
        let config = gevrek_model::Config::load(&gevrek_model::paths::config_file()).ok();
        // The wallpaper is read beside the lock, which does not wait for it.
        if let Some(file) = wallpaper_file(config.as_ref()) {
            let read = shell.sender();
            std::thread::Builder::new()
                .name("gevrek-lock-wallpaper".to_string())
                .spawn(move || match image::open(&file) {
                    Ok(picture) => {
                        let picture = if picture.width().max(picture.height()) > WALLPAPER_SIDE {
                            picture.thumbnail(WALLPAPER_SIDE, WALLPAPER_SIDE)
                        } else {
                            picture
                        }
                        .to_rgba8();
                        let size = [picture.width() as usize, picture.height() as usize];
                        read.send(Message::Wallpaper(
                            egui::ColorImage::from_rgba_unmultiplied(size, picture.as_raw()),
                        ));
                    }
                    Err(error) => {
                        log::warn!("the wallpaper {} was not read: {error}", file.display())
                    }
                })?;
        }
        // The accent: what the compositor says the desktop has now
        // (`GEVREK_LOCK_ACCENT`, as `rrggbb`: it may be the wallpaper's
        // colour), else the configuration's own.
        let told = std::env::var("GEVREK_LOCK_ACCENT")
            .ok()
            .and_then(|text| accent_told(&text));
        let accent = told
            .or_else(|| {
                config
                    .as_ref()
                    .map(|config| config.desktop.appearance.active_border_color)
            })
            .map_or(
                egui::Color32::from_rgb(64, 224, 208),
                |(red, green, blue)| egui::Color32::from_rgb(red, green, blue),
            );
        let state = Rc::new(RefCell::new(State {
            password: String::new(),
            status: Status::Waiting,
            user,
            settings: {
                let mut settings = gevrek_ui::Settings::load(&appearance);
                // The desktop's high contrast is the lock screen's too.
                if config
                    .as_ref()
                    .is_some_and(|config| config.desktop.appearance.high_contrast)
                {
                    settings.preset = gevrek_ui::theme::Preset::HighContrastDark;
                }
                settings
            },
            accent,
            check: to_check,
            unlocked: false,
            greeter,
            wallpaper: None,
            wallpaper_since: None,
            accent_of_wallpaper: std::env::var("GEVREK_LOCK_ACCENT")
                .is_ok_and(|told| told.trim() == "wallpaper"),
            layouts: layouts_known(config.as_ref()),
            tell_shown: shown_socket,
        }));

        let shared = Rc::clone(&state);
        shell.lock(move || -> Draw {
            let state = Rc::clone(&shared);
            let mut fonts = false;
            Box::new(move |ui, control| {
                let mut state = state.borrow_mut();
                if state.unlocked {
                    control.unlock();
                    return Flow::Continue;
                }
                draw(&mut state, ui, &mut fonts, control.keyboard_layout());
                if let Some(socket) = state.tell_shown.take() {
                    // (Beside the drawing: the answer is not waited for,
                    // and a service that knows no such word says no.)
                    let _ = std::thread::Builder::new()
                        .name("gevrek-lock-shown".to_string())
                        .spawn(move || {
                            let _ = ask(&socket, &gevrek_dm::wire::Ask::Shown);
                        });
                }
                Flow::Continue
            })
        });
        let verdict_state = Rc::clone(&state);
        shell.run(false, move |message, control| {
            let mut state = verdict_state.borrow_mut();
            match message {
                // The login stays: the display manager shows the
                // session's console now, and this one again afterwards.
                Message::Verdict(Ok(())) if state.greeter => state.status = Status::Waiting,
                Message::Verdict(Ok(())) => {
                    state.unlocked = true;
                    control.unlock();
                }
                Message::Verdict(Err(why)) => state.status = Status::Wrong(Instant::now(), why),
                Message::Wallpaper(picture) => {
                    // The accent is the picture's, where the desktop's is
                    // (said so by whoever started this: the login).
                    if state.accent_of_wallpaper {
                        let pixels = picture
                            .pixels
                            .iter()
                            .step_by((picture.pixels.len() / 40_000).max(1))
                            .map(|pixel| (pixel.r(), pixel.g(), pixel.b()));
                        if let Some((red, green, blue)) =
                            gevrek_model::theme::accent_of_pixels(pixels)
                        {
                            state.accent = egui::Color32::from_rgb(red, green, blue);
                        }
                    }
                    state.wallpaper = Some(picture);
                    state.wallpaper_since = Some(Instant::now());
                }
            }
            control.redraw_all();
        })?;
        Ok(())
    }

    /// How much of the wallpaper is there, so long after it was read:
    /// it comes in over a third of a second.
    pub fn wallpaper_shown(since: Option<Duration>) -> f32 {
        const COMING: f32 = 0.35;
        let part = since.map_or(1.0, |since| (since.as_secs_f32() / COMING).clamp(0.0, 1.0));
        part * part * (3.0 - 2.0 * part)
    }

    /// How strong the caret is at a moment, from 0 to 1: it breathes, a
    /// soft swell and fade and no hard blink.
    pub fn caret_breath(seconds: f64) -> f32 {
        const BREATH: f64 = 1.6;
        let part = (seconds.rem_euclid(BREATH)) / BREATH;
        (0.5 + 0.5 * (part * std::f64::consts::TAU).cos()) as f32
    }

    /// Where a place in a field's text is on the screen. (The text stands
    /// in the middle of its field: its line is laid out around nought,
    /// and the field says where the line's left end is.)
    fn place_in(
        output: &egui::text_edit::TextEditOutput,
        cursor: egui::text::CCursor,
    ) -> egui::Rect {
        output
            .galley
            .pos_from_cursor(cursor)
            .translate(output.galley_pos.to_vec2())
            .translate(egui::vec2(-output.galley.rect.left(), 0.0))
    }

    /// A soft glow around a small shape, and the shape itself: the look
    /// of the caret, and of the dots that stand for a password's letters.
    fn glowing(painter: &egui::Painter, shape: egui::Rect, colour: egui::Color32, strength: f32) {
        let round = shape.height().min(shape.width()) / 2.0;
        for (grow, part) in [(6.0, 0.05), (4.0, 0.10), (2.0, 0.20)] {
            painter.rect_filled(
                shape.expand(grow),
                round + grow,
                colour.gamma_multiply(part * strength),
            );
        }
        painter.rect_filled(shape, round, colour.gamma_multiply(0.3 + 0.7 * strength));
    }

    /// How far the dots of a password stand apart, and how large one is.
    const DOT_STEP: f32 = 14.0;
    const DOT: f32 = 7.0;

    /// Where the left edge of a password's `index`th dot is, of `letters`
    /// that stand in the middle of the field. (Its dots are gevrek's own,
    /// further apart than the toolkit's: their places are counted here,
    /// and the caret goes by the same count.)
    pub fn dot_left(middle: f32, letters: usize, index: usize) -> f32 {
        middle - letters as f32 * DOT_STEP / 2.0 + index as f32 * DOT_STEP
    }

    /// The caret of the fields: a short line under the place where the
    /// next letter goes, with a soft glow around it, breathing. It is
    /// where the typing is: after what was typed, and wherever the arrow
    /// keys put it. `dots`: the field shows a password of so many letters.
    fn glow_caret(
        ui: &egui::Ui,
        output: &egui::text_edit::TextEditOutput,
        dots: Option<usize>,
        colour: egui::Color32,
    ) {
        if !output.response.response.has_focus() {
            return;
        }
        let Some(range) = output.cursor_range else {
            return;
        };
        let at = place_in(output, range.primary);
        let left = match dots {
            Some(letters) => dot_left(
                output.response.response.rect.center().x,
                letters,
                range.primary.index.0.min(letters),
            ),
            None => at.left() + 1.0,
        };
        let breath = caret_breath(ui.input(|input| input.time));
        let line =
            egui::Rect::from_min_size(egui::pos2(left, at.bottom() + 1.0), egui::vec2(11.0, 2.0));
        // (Beyond the field's own edge: a glow is not cut off by it.)
        let painter = ui.painter().with_clip_rect(egui::Rect::EVERYTHING);
        glowing(&painter, line, colour, 0.4 + 0.6 * breath);
        // (At the display's own rate: thirty pictures a second, as it
        // was, is seen as a caret that does not breathe evenly.)
        ui.ctx().request_repaint();
    }

    /// The letters of a password, each a dot in the caret's own look: the
    /// accent's colour with a soft glow, and no breathing (they are what
    /// is there, the caret is where the next one goes).
    fn glow_dots(
        ui: &egui::Ui,
        output: &egui::text_edit::TextEditOutput,
        letters: usize,
        colour: egui::Color32,
    ) {
        let rect = output.response.response.rect;
        let painter = ui.painter();
        for index in 0..letters {
            let left = dot_left(rect.center().x, letters, index);
            // (More than the field holds are not drawn beyond its edge.)
            if left < rect.left() || left + DOT_STEP > rect.right() {
                continue;
            }
            let dot = egui::Rect::from_min_size(
                egui::pos2(left + (11.0 - DOT) / 2.0, rect.center().y - DOT / 2.0),
                egui::vec2(DOT, DOT),
            );
            glowing(painter, dot, colour, 0.6);
        }
    }

    /// A field of the login and the lock: rounded, a little lighter than
    /// what it lies on, with the caret above in place of the toolkit's.
    fn field(
        ui: &mut egui::Ui,
        id: egui::Id,
        text: &mut String,
        hint: &str,
        password: bool,
        enabled: bool,
        accent: egui::Color32,
    ) -> egui::Response {
        let focused = ui.memory(|memory| memory.has_focus(id));
        let edge = if focused {
            accent.gamma_multiply(0.75)
        } else {
            egui::Color32::from_white_alpha(26)
        };
        egui::Frame::new()
            .fill(egui::Color32::from_white_alpha(if enabled {
                16
            } else {
                8
            }))
            .stroke(egui::Stroke::new(1.0, edge))
            .corner_radius(14)
            .inner_margin(egui::Margin::symmetric(14, 9))
            .show(ui, |ui| {
                // (Its own caret is drawn below.)
                ui.visuals_mut().text_cursor.stroke = egui::Stroke::NONE;
                let mut edit = egui::TextEdit::singleline(text)
                    .id(id)
                    .password(password)
                    .hint_text(hint)
                    .frame(egui::Frame::NONE)
                    .interactive(enabled)
                    .font(egui::FontId::proportional(17.0))
                    .desired_width(252.0)
                    .align(egui::Align2::CENTER_TOP);
                if password {
                    // (The toolkit's own dots keep their places and are
                    // not seen: the dots drawn below stand there.)
                    edit = edit.text_color(egui::Color32::TRANSPARENT);
                }
                let output = edit.show(ui);
                if password {
                    // What was typed into a password's field is not kept
                    // to be taken back: the toolkit remembers a field's
                    // earlier text for "undo", and the greeter lives on
                    // after a person has logged in and out again. Whoever
                    // came next could bring that person's password back
                    // into the field with one chord, and press Enter.
                    if let Some(mut kept) = egui::TextEdit::load_state(ui.ctx(), id) {
                        kept.clear_undoer();
                        egui::TextEdit::store_state(ui.ctx(), id, kept);
                    }
                }
                let dots = password.then(|| text.chars().count());
                if let Some(letters) = dots {
                    glow_dots(ui, &output, letters, accent);
                }
                glow_caret(ui, &output, dots, accent);
                output.response.response
            })
            .inner
    }

    /// Dark towards the corners: what is in the middle is what is looked at.
    fn vignette(painter: &egui::Painter, area: egui::Rect) {
        let dark = |alpha: u8| egui::Color32::from_black_alpha(alpha);
        // Nine points: the corners darkest, the middle clear.
        let strengths = [[150, 70, 150], [70, 0, 70], [150, 70, 150]];
        let mut mesh = egui::Mesh::default();
        for (row, line) in strengths.iter().enumerate() {
            for (column, alpha) in line.iter().enumerate() {
                mesh.colored_vertex(
                    egui::pos2(
                        area.left() + area.width() * column as f32 / 2.0,
                        area.top() + area.height() * row as f32 / 2.0,
                    ),
                    dark(*alpha),
                );
            }
        }
        for row in 0..2u32 {
            for column in 0..2u32 {
                let first = row * 3 + column;
                // (Each quarter is cut towards the middle, so that the
                // dark runs evenly into it.)
                if row == column {
                    mesh.add_triangle(first, first + 1, first + 4);
                    mesh.add_triangle(first, first + 4, first + 3);
                } else {
                    mesh.add_triangle(first, first + 1, first + 3);
                    mesh.add_triangle(first + 1, first + 4, first + 3);
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    /// The first letter of a name, for the round sign above it.
    pub fn initial(name: &str) -> Option<String> {
        name.trim()
            .chars()
            .find(|letter| letter.is_alphanumeric())
            .map(|letter| letter.to_uppercase().collect())
    }

    /// The keyboard layout to name: the one in use among those known,
    /// else the first; nothing where none is known.
    pub fn layout_shown(layouts: &[String], in_use: u32) -> Option<&str> {
        layouts
            .get(in_use as usize)
            .or_else(|| layouts.first())
            .map(String::as_str)
    }

    fn draw(state: &mut State, ui: &mut egui::Ui, fonts: &mut bool, layout_in_use: u32) {
        let context = ui.ctx().clone();
        if !*fonts {
            theme::install_fonts(&context, state.settings.system_font);
            *fonts = true;
        }
        let palette = state.settings.resolve(egui::Theme::Dark, state.accent);
        theme::apply(&context, &palette, &state.settings);
        let accent = state.accent;
        let now = jiff::Zoned::now();
        // The clock moves on.
        context.request_repaint_after(Duration::from_secs(1));
        let mut submit = false;
        let mut asked: Option<&str> = None;
        let mut next_layout = false;
        // (Every display draws for itself, so each has the picture as
        // something of its own to draw, kept with its drawing's memory.)
        let texture_key = egui::Id::unique("gevrek lock wallpaper");
        let mut wallpaper = context.data(|data| data.get_temp::<egui::TextureHandle>(texture_key));
        if wallpaper.is_none() {
            if let Some(picture) = &state.wallpaper {
                let texture = context.load_texture(
                    "lock-wallpaper",
                    picture.clone(),
                    egui::TextureOptions::LINEAR,
                );
                context.data_mut(|data| data.insert_temp(texture_key, texture.clone()));
                wallpaper = Some(texture);
            }
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(palette.bg))
            .show(ui, |ui| {
                let area = ui.max_rect();
                // gevrek's mark, small, in the lower left corner: whose
                // lock screen (and whose login) this is.
                if let Some(mark) = gevrek_ui::brand::mark(&context, true, 26.0) {
                    let over = context.layer_painter(egui::LayerId::new(
                        egui::Order::Foreground,
                        egui::Id::unique("gevrek's mark on the lock screen"),
                    ));
                    over.image(
                        mark.id(),
                        egui::Rect::from_center_size(
                            egui::pos2(area.left() + 34.0, area.bottom() - 34.0),
                            egui::vec2(26.0, 26.0),
                        ),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::from_white_alpha(200),
                    );
                }
                // The wallpaper, covering the display, under a veil of
                // the background's colour so that the words stay readable.
                if let Some(texture) = &wallpaper {
                    let size = texture.size_vec2();
                    let (from, to) = covering((size.x, size.y), (area.width(), area.height()));
                    // (It was read beside the lock and comes a moment
                    // after the first frame: in over a third of a second.)
                    let there = wallpaper_shown(state.wallpaper_since.map(|since| since.elapsed()));
                    if there < 1.0 {
                        ui.ctx().request_repaint();
                    }
                    ui.painter().image(
                        texture.id(),
                        area,
                        egui::Rect::from_min_max(from.into(), to.into()),
                        egui::Color32::WHITE.gamma_multiply(there),
                    );
                    ui.painter()
                        .rect_filled(area, 0.0, palette.bg.gamma_multiply(0.62 * there));
                }
                vignette(ui.painter(), area);
                let checking = state.status == Status::Checking;
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() * 0.15).max(16.0));
                    ui.label(
                        egui::RichText::new(now.strftime("%H:%M").to_string())
                            .size(104.0)
                            .color(palette.text),
                    );
                    ui.label(
                        egui::RichText::new(now.strftime("%A, %-d %B").to_string())
                            .size(20.0)
                            .color(palette.text.gamma_multiply(0.72)),
                    );
                    ui.add_space(38.0);
                    // The card: who, and the password.
                    ui.allocate_ui_with_layout(
                        egui::vec2(340.0, 0.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            egui::Frame::new()
                                .fill(palette.bg.gamma_multiply(0.66))
                                .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(20)))
                                .corner_radius(26)
                                .inner_margin(egui::Margin::symmetric(26, 24))
                                .shadow(egui::Shadow {
                                    offset: [0, 14],
                                    blur: 44,
                                    spread: 0,
                                    color: egui::Color32::from_black_alpha(120),
                                })
                                .show(ui, |ui| {
                                    ui.set_width(284.0);
                                    ui.vertical_centered(|ui| {
                                        card(ui, state, &palette, accent, checking, &mut submit);
                                    });
                                });
                        },
                    );
                    ui.add_space(24.0);
                    // Under the card: the keyboard's layout, and (at the
                    // login) switching the machine off and starting it again.
                    let layout = layout_shown(&state.layouts, layout_in_use).map(str::to_string);
                    let things = usize::from(layout.is_some()) + if state.greeter { 2 } else { 0 };
                    if things > 0 {
                        let wide = things as f32 * 44.0
                            + (things.saturating_sub(1)) as f32 * 12.0
                            + if layout.is_some() { 34.0 } else { 0.0 };
                        ui.allocate_ui_with_layout(
                            egui::vec2(wide, 44.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                let round = |sign: String| {
                                    egui::Button::new(
                                        egui::RichText::new(sign).size(17.0).color(palette.text),
                                    )
                                    .fill(egui::Color32::from_white_alpha(18))
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_white_alpha(22),
                                    ))
                                    .corner_radius(22)
                                    .min_size(egui::vec2(44.0, 44.0))
                                };
                                if let Some(layout) = &layout {
                                    let sign = format!("{}  {layout}", icons::KEYBOARD);
                                    let several = state.layouts.len() > 1;
                                    let hint = if several && state.greeter {
                                        "The keyboard's layout. A press, or Alt+Shift, goes to the next one."
                                    } else if several {
                                        "The keyboard's layout. Alt+Shift goes to the next one."
                                    } else {
                                        "The keyboard's layout."
                                    };
                                    let pressed = ui
                                        .add_enabled(!checking, round(sign))
                                        .on_hover_text(hint)
                                        .clicked();
                                    if pressed && several && state.greeter {
                                        next_layout = true;
                                    }
                                }
                                if state.greeter {
                                    if ui
                                        .add_enabled(!checking, round(icons::POWER.to_string()))
                                        .on_hover_text("Shut down")
                                        .clicked()
                                    {
                                        asked = Some("poweroff");
                                    }
                                    if ui
                                        .add_enabled(
                                            !checking,
                                            round(icons::ARROW_CLOCKWISE.to_string()),
                                        )
                                        .on_hover_text("Restart")
                                        .clicked()
                                    {
                                        asked = Some("restart");
                                    }
                                }
                            },
                        );
                    }
                });
            });
        if next_layout {
            next_keyboard_layout();
        }
        if let Some(what) = asked {
            state.status = Status::Checking;
            let _ = state.check.send(format!("\n{what}"));
        }
        if submit {
            let mut password = std::mem::take(&mut state.password);
            if state.greeter {
                // The name and the password, as the checking thread
                // takes them apart (a name has no line break in it).
                let mut both = format!("{}\n{password}", state.user.trim());
                std::mem::swap(&mut both, &mut password);
                forget(&mut both);
            }
            state.status = Status::Checking;
            if state.check.send(password).is_err() {
                state.status = Status::Wrong(
                    Instant::now(),
                    "The check could not be started.".to_string(),
                );
            }
        }
    }

    /// What is on the card: a round sign with the name's first letter, the
    /// name (asked for at the login), the password, and a line that says
    /// what is going on.
    fn card(
        ui: &mut egui::Ui,
        state: &mut State,
        palette: &theme::Palette,
        accent: egui::Color32,
        checking: bool,
        submit: &mut bool,
    ) {
        // The round sign.
        let (place, _) = ui.allocate_exact_size(egui::vec2(68.0, 68.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.circle_filled(place.center(), 38.0, accent.gamma_multiply(0.10));
        painter.circle_filled(place.center(), 34.0, accent.gamma_multiply(0.92));
        let on_accent = theme::readable(egui::Color32::BLACK, accent, 4.5);
        let sign = initial(&state.user).unwrap_or_else(|| icons::USER.to_string());
        painter.text(
            place.center(),
            egui::Align2::CENTER_CENTER,
            sign,
            egui::FontId::proportional(28.0),
            on_accent,
        );
        ui.add_space(14.0);
        let field_id = egui::Id::unique("gevrek lock password");
        let name_id = egui::Id::unique("gevrek greeter name");
        if state.greeter {
            let name = field(
                ui,
                name_id,
                &mut state.user,
                "Name",
                false,
                !checking,
                accent,
            );
            // Enter in the name goes on to the password.
            if name.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                ui.memory_mut(|memory| memory.request_focus(field_id));
            }
            // One of the two fields has the keys: the name while there
            // is none, then the password.
            if !checking
                && !ui.memory(|memory| memory.has_focus(name_id) || memory.has_focus(field_id))
            {
                let first = if state.user.is_empty() {
                    name_id
                } else {
                    field_id
                };
                ui.memory_mut(|memory| memory.request_focus(first));
            }
            ui.add_space(10.0);
        } else {
            ui.label(
                egui::RichText::new(&state.user)
                    .size(18.0)
                    .strong()
                    .color(palette.text),
            );
            ui.add_space(12.0);
            if !checking {
                ui.memory_mut(|memory| memory.request_focus(field_id));
            }
        }
        let password = field(
            ui,
            field_id,
            &mut state.password,
            "Password",
            true,
            !checking,
            accent,
        );
        if password.changed() {
            // Typing again takes the refusal away.
            if matches!(state.status, Status::Wrong(..)) {
                state.status = Status::Waiting;
            }
        }
        if !checking
            && (password.lost_focus() || !state.greeter)
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
            && !state.password.is_empty()
            && !(state.greeter && state.user.trim().is_empty())
        {
            *submit = true;
        }
        ui.add_space(12.0);
        match &state.status {
            Status::Checking => {
                ui.label(egui::RichText::new("Checking…").color(palette.muted));
            }
            Status::Wrong(_, why) => {
                ui.label(egui::RichText::new(why).color(palette.error));
            }
            Status::Waiting => {
                ui.label(
                    egui::RichText::new(if state.greeter {
                        "Enter logs in"
                    } else {
                        "Enter unlocks"
                    })
                    .color(palette.faint)
                    .size(12.5),
                );
            }
        }
    }

    /// Ask the compositor the login runs in for the next keyboard layout
    /// (it is this program's parent, there for nothing else, and takes
    /// the signal as that).
    fn next_keyboard_layout() {
        // SAFETY: a signal to the process that started this one.
        unsafe {
            libc::kill(libc::getppid(), libc::SIGUSR2);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_password_is_overwritten_when_it_is_forgotten() {
            let mut password = String::from("correct horse");
            let before = password.as_ptr();
            let length = password.len();
            forget(&mut password);
            assert!(password.is_empty());
            // The memory it was in holds nothing of it.
            // SAFETY: the buffer is still the string's own (clear keeps
            // the capacity), and `length` bytes of it were written.
            let bytes = unsafe { std::slice::from_raw_parts(before, length) };
            assert!(bytes.iter().all(|byte| *byte == 0));
        }

        #[test]
        fn a_password_for_nobody_is_refused_by_the_system() {
            // An account that does not exist: the system says no (or has
            // no service to ask, which is a refusal too). Never a yes.
            // (Not the real user: wrong tries may count against them.)
            assert!(check("gevrek-no-such-user", "not-a-password").is_err());
        }

        #[test]
        fn the_keyboards_layout_is_named_as_the_one_in_use() {
            let layouts = layouts_said("tr, us");
            assert_eq!(layouts, ["TR", "US"]);
            assert_eq!(layout_shown(&layouts, 0), Some("TR"));
            assert_eq!(layout_shown(&layouts, 1), Some("US"));
            // A place there is no name for: the first is better than none.
            assert_eq!(layout_shown(&layouts, 7), Some("TR"));
            assert!(layouts_said(" , ").is_empty());
            assert_eq!(layout_shown(&[], 0), None);
        }

        #[test]
        fn a_passwords_dots_stand_in_the_middle_and_the_caret_after_them() {
            // Four letters around 200: two dots on each side, the caret's
            // place (the fifth) right of the last; none: in the middle.
            assert_eq!(dot_left(200.0, 4, 0), 172.0);
            assert_eq!(dot_left(200.0, 4, 3), 214.0);
            assert_eq!(dot_left(200.0, 4, 4), 228.0);
            assert_eq!(dot_left(200.0, 0, 0), 200.0);
            // Between the second and the third after two presses of Left.
            assert_eq!(dot_left(200.0, 4, 2), 200.0);
        }

        #[test]
        fn the_caret_breathes_and_does_not_blink() {
            // Whole at the start of a breath, gone in the middle, and
            // never a jump between two moments close together.
            assert!(caret_breath(0.0) > 0.99);
            assert!(caret_breath(0.8) < 0.01);
            let mut before = caret_breath(0.0);
            for step in 1..=160 {
                let now = caret_breath(f64::from(step) / 100.0);
                assert!((now - before).abs() < 0.03, "a jump at {step}");
                before = now;
            }
        }

        #[test]
        fn the_wallpaper_comes_in_and_does_not_jump() {
            assert_eq!(wallpaper_shown(Some(Duration::ZERO)), 0.0);
            let half = wallpaper_shown(Some(Duration::from_millis(175)));
            assert!(half > 0.3 && half < 0.7);
            assert_eq!(wallpaper_shown(Some(Duration::from_secs(2))), 1.0);
            assert_eq!(wallpaper_shown(None), 1.0);
        }

        #[test]
        fn the_round_sign_has_the_names_first_letter() {
            assert_eq!(initial("ada").as_deref(), Some("A"));
            assert_eq!(initial("  élan").as_deref(), Some("É"));
            assert_eq!(initial(""), None);
            assert_eq!(initial(" _ "), None);
        }

        #[test]
        fn an_accent_is_told_as_six_letters() {
            assert_eq!(accent_told("e63939"), Some((230, 57, 57)));
            assert_eq!(accent_told("#00AAff"), Some((0, 170, 255)));
            assert_eq!(accent_told(""), None);
            assert_eq!(accent_told("e639"), None);
            assert_eq!(accent_told("zzzzzz"), None);
        }

        #[test]
        fn the_wallpaper_covers_a_display_of_another_shape() {
            // The same shape: all of it.
            assert_eq!(
                covering((1920.0, 1080.0), (1280.0, 720.0)),
                ((0.0, 0.0), (1.0, 1.0))
            );
            // A wide picture on a square display: its middle half.
            assert_eq!(
                covering((200.0, 100.0), (50.0, 50.0)),
                ((0.25, 0.0), (0.75, 1.0))
            );
            // A square picture on a wide display: its middle band.
            assert_eq!(
                covering((100.0, 100.0), (200.0, 100.0)),
                ((0.0, 0.25), (1.0, 0.75))
            );
            assert_eq!(covering((0.0, 0.0), (10.0, 10.0)), ((0.0, 0.0), (1.0, 1.0)));
        }

        #[test]
        fn a_service_of_the_system_is_named() {
            assert!(!service().is_empty());
        }
    }
}
