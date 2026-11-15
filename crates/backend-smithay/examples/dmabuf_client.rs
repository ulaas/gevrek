//! A client that hands the compositor a buffer of the graphics card
//! (`linux-dmabuf`) instead of shared memory: a 256x256 window, all magenta.
//! For the check of `linux-dmabuf` on a machine without a program that
//! draws with the card: the buffer is a "dumb buffer" made on the display
//! device itself (`/dev/dri/card0`, or `DMABUF_CARD`), which every display
//! driver has.
//!
//!     WAYLAND_DISPLAY=wayland-1 dmabuf_client

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("dmabuf_client: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
    use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, QueueHandle};
    use wayland_protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1, zwp_linux_dmabuf_v1,
    };
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

    const SIZE: u32 = 256;
    /// `DRM_FORMAT_XRGB8888`: bytes blue, green, red, unused.
    const XRGB8888: u32 = 0x3432_5258;
    const MAGENTA: [u8; 4] = [0xff, 0x00, 0xff, 0xff];
    const LINEAR: u64 = 0;
    const INVALID: u64 = 0x00ff_ffff_ffff_ffff;

    // The kernel's structures (drm_mode.h, drm.h) and their ioctl numbers.
    #[repr(C)]
    struct CreateDumb {
        height: u32,
        width: u32,
        bpp: u32,
        flags: u32,
        handle: u32,
        pitch: u32,
        size: u64,
    }
    #[repr(C)]
    struct MapDumb {
        handle: u32,
        pad: u32,
        offset: u64,
    }
    #[repr(C)]
    struct PrimeHandle {
        handle: u32,
        flags: u32,
        fd: i32,
    }
    const MODE_CREATE_DUMB: u64 = 0xC020_64B2;
    const MODE_MAP_DUMB: u64 = 0xC010_64B3;
    const PRIME_HANDLE_TO_FD: u64 = 0xC00C_642D;

    fn ioctl<T>(fd: i32, request: u64, argument: &mut T) -> std::io::Result<()> {
        // SAFETY: `argument` is the structure this request reads and writes.
        if unsafe { libc::ioctl(fd, request as _, argument as *mut T) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// A magenta buffer on the display device, as a dmabuf, and the length
    /// of its rows in bytes.
    fn dumb_buffer() -> std::io::Result<(OwnedFd, u32)> {
        let path = std::env::var("DMABUF_CARD").unwrap_or_else(|_| "/dev/dri/card0".to_string());
        let card = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let fd = card.as_raw_fd();
        let mut create = CreateDumb {
            height: SIZE,
            width: SIZE,
            bpp: 32,
            flags: 0,
            handle: 0,
            pitch: 0,
            size: 0,
        };
        ioctl(fd, MODE_CREATE_DUMB, &mut create)?;
        let mut map = MapDumb {
            handle: create.handle,
            pad: 0,
            offset: 0,
        };
        ioctl(fd, MODE_MAP_DUMB, &mut map)?;
        let length = create.size as usize;
        // SAFETY: the kernel gave this offset for mapping the buffer.
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                length,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                map.offset as libc::off_t,
            )
        };
        if memory == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `memory` is `length` bytes, mapped just above.
        let pixels = unsafe { std::slice::from_raw_parts_mut(memory.cast::<u8>(), length) };
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&MAGENTA);
        }
        // SAFETY: mapped above with this length.
        unsafe { libc::munmap(memory, length) };
        let mut prime = PrimeHandle {
            handle: create.handle,
            flags: (libc::O_CLOEXEC | libc::O_RDWR) as u32,
            fd: -1,
        };
        ioctl(fd, PRIME_HANDLE_TO_FD, &mut prime)?;
        // SAFETY: the kernel returned a new descriptor that is ours.
        Ok((unsafe { OwnedFd::from_raw_fd(prime.fd) }, create.pitch))
    }

    #[derive(Default)]
    struct App {
        compositor: Option<wl_compositor::WlCompositor>,
        wm_base: Option<xdg_wm_base::XdgWmBase>,
        dmabuf: Option<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
        /// The modifiers the compositor takes for XRGB8888.
        modifiers: Vec<u64>,
        surface: Option<wl_surface::WlSurface>,
        configured: bool,
        attached: bool,
        closed: bool,
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::connect_to_env()?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut app = App::default();
        // Once for the globals, once more for the formats of linux-dmabuf.
        queue.roundtrip(&mut app)?;
        queue.roundtrip(&mut app)?;
        let compositor = app.compositor.clone().ok_or("no wl_compositor")?;
        let wm_base = app.wm_base.clone().ok_or("no xdg_wm_base")?;
        let dmabuf = app
            .dmabuf
            .clone()
            .ok_or("the compositor does not offer linux-dmabuf")?;

        let surface = compositor.create_surface(&handle, ());
        let xdg = wm_base.get_xdg_surface(&surface, &handle, ());
        let toplevel = xdg.get_toplevel(&handle, ());
        toplevel.set_title("chk-dmabuf".to_string());
        toplevel.set_app_id("chk-dmabuf".to_string());
        surface.commit();
        app.surface = Some(surface.clone());
        while !app.configured {
            queue.blocking_dispatch(&mut app)?;
        }

        let (fd, pitch) = dumb_buffer()?;
        let modifier = if app.modifiers.contains(&LINEAR) {
            LINEAR
        } else if app.modifiers.contains(&INVALID) || app.modifiers.is_empty() {
            INVALID
        } else {
            app.modifiers[0]
        };
        let params = dmabuf.create_params(&handle, ());
        params.add(
            fd.as_fd(),
            0,
            0,
            pitch,
            (modifier >> 32) as u32,
            modifier as u32,
        );
        let buffer = params.create_immed(
            SIZE as i32,
            SIZE as i32,
            XRGB8888,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &handle,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, i32::MAX, i32::MAX);
        surface.commit();
        app.attached = true;
        // A buffer the compositor refuses ends the connection with an error.
        queue.roundtrip(&mut app)?;
        println!(
            "attached a {SIZE}x{SIZE} dmabuf (rows of {pitch} bytes, modifier {modifier:#x}; the compositor takes {:?})",
            app.modifiers
        );
        while !app.closed {
            queue.blocking_dispatch(&mut app)?;
        }
        Ok(())
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for App {
        fn event(
            app: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            handle: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_compositor" => {
                        app.compositor = Some(registry.bind(name, version.min(4), handle, ()));
                    }
                    "xdg_wm_base" => {
                        app.wm_base = Some(registry.bind(name, version.min(2), handle, ()));
                    }
                    // Version 3 lists the formats and modifiers as events.
                    "zwp_linux_dmabuf_v1" => {
                        app.dmabuf = Some(registry.bind(name, version.min(3), handle, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for App {
        fn event(
            app: &mut Self,
            _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
            event: zwp_linux_dmabuf_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwp_linux_dmabuf_v1::Event::Modifier {
                format: XRGB8888,
                modifier_hi,
                modifier_lo,
            } = event
            {
                app.modifiers
                    .push((u64::from(modifier_hi) << 32) | u64::from(modifier_lo));
            }
        }
    }

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
        fn event(
            _: &mut Self,
            wm_base: &xdg_wm_base::XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                wm_base.pong(serial);
            }
        }
    }

    impl Dispatch<xdg_surface::XdgSurface, ()> for App {
        fn event(
            app: &mut Self,
            xdg_surface: &xdg_surface::XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                xdg_surface.ack_configure(serial);
                app.configured = true;
                if app.attached {
                    if let Some(surface) = &app.surface {
                        surface.commit();
                    }
                }
            }
        }
    }

    impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
        fn event(
            app: &mut Self,
            _: &xdg_toplevel::XdgToplevel,
            event: xdg_toplevel::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_toplevel::Event::Close = event {
                app.closed = true;
            }
        }
    }

    macro_rules! ignore_events {
        ($($interface:ty),* $(,)?) => {
            $(
                impl Dispatch<$interface, ()> for App {
                    fn event(
                        _: &mut Self,
                        _: &$interface,
                        _: <$interface as wayland_client::Proxy>::Event,
                        _: &(),
                        _: &Connection,
                        _: &QueueHandle<Self>,
                    ) {
                    }
                }
            )*
        };
    }

    ignore_events!(
        wl_compositor::WlCompositor,
        wl_surface::WlSurface,
        wl_buffer::WlBuffer,
        zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1,
    );
}
