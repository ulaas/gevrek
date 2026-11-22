//! Borders of every style around empty places of the desktop, for a few
//! seconds, to look at: `cargo run -p gevrek-macos --example borders -- 5`.

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use gevrek_engine::workspace::BorderLook;
    use gevrek_engine::{Rect, WindowHandle};
    use gevrek_macos::borders::Borders;
    use gevrek_model::graph::BorderStyle;
    use objc2::MainThreadMarker;

    let seconds: f64 = std::env::args()
        .nth(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(5.0);
    let mtm = MainThreadMarker::new().expect("on the main thread");
    let mut borders = Borders::new(mtm);
    let styles = [
        BorderStyle::Square,
        BorderStyle::Rounded,
        BorderStyle::Double,
        BorderStyle::Dashed,
    ];
    for (index, style) in styles.into_iter().enumerate() {
        let look = BorderLook {
            enabled: true,
            width: 6,
            offset: 0,
            style,
            active_color: (64, 224, 208),
            inactive_color: (128, 128, 128),
            active_alpha: 255,
            inactive_alpha: 255,
            floating: false,
        };
        // (No window of that number: each border stands on its own.)
        let around = Rect::new(120 + 300 * index as i32, 300, 240, 160);
        borders.draw(
            WindowHandle(0x7fff_0000 + index as isize),
            around,
            &look,
            true,
        );
        println!("{style:?} around {around:?}");
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    while std::time::Instant::now() < until {
        // SAFETY: this thread's run loop, for a moment.
        unsafe {
            objc2_core_foundation::CFRunLoop::run_in_mode(
                objc2_core_foundation::kCFRunLoopDefaultMode,
                0.1,
                false,
            )
        };
    }
}
