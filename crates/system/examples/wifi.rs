//! What the wifi's side says, asked twice a few seconds apart, the main
//! thread's loop running between (where the location is asked for):
//! `cargo run -p gevrek-system --example wifi -- 8`.
fn main() {
    // An application as the shell is (one that is never in front): the
    // system asks its questions of an application.
    #[cfg(target_os = "macos")]
    unsafe {
        use objc2::runtime::AnyObject;
        use objc2::{class, msg_send};
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let _: bool = msg_send![app, setActivationPolicy: 1isize];
        let _: () = msg_send![app, finishLaunching];
    }
    let seconds: f64 = std::env::args().nth(1).and_then(|text| text.parse().ok()).unwrap_or(8.0);
    println!("first: {:?}", gevrek_system::wifi::read().map(|wifi| (wifi.enabled, wifi.networks.len())));
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            static kCFRunLoopDefaultMode: *const std::ffi::c_void;
            fn CFRunLoopRunInMode(mode: *const std::ffi::c_void, seconds: f64, after_source: bool) -> i32;
        }
        // SAFETY: this thread's run loop, for a while.
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false) };
    }
    gevrek_system::wifi::rescan();
    match gevrek_system::wifi::read() {
        Some(wifi) => {
            println!("then: on {}, {} network(s)", wifi.enabled, wifi.networks.len());
            for network in wifi.networks.iter().take(8) {
                println!(
                    "  {}{} {}% {}{}",
                    if network.in_use { "* " } else { "  " },
                    network.name,
                    network.signal,
                    if network.secured { "secured" } else { "open" },
                    if network.known { ", known" } else { "" }
                );
            }
        }
        None => println!("then: no wifi"),
    }
}
