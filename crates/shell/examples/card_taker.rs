//! A program that takes a shared stream in buffers of the graphics
//! card's own memory, as a meeting program does that draws with the
//! card: for the check of screen sharing without a copy.
//!
//!     card_taker NODE
//!
//! It takes the stream of that node in PipeWire, offering only buffers
//! of the card in the layouts the compositor takes, and says of each
//! buffer what kind it is. Such a buffer cannot be read here, so the
//! first one is shown as the content of a window (and kept: the stream
//! does not draw into it again); the later ones are counted.

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
mod taker {
    use pipewire as pw;
    use pw::spa;
    use std::cell::RefCell;
    use std::os::fd::BorrowedFd;
    use std::rc::Rc;
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
    use wayland_protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
        zwp_linux_dmabuf_v1::{self, ZwpLinuxDmabufV1},
    };
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

    /// `XRGB8888`, as the graphics card's buffers name it.
    const XRGB: u32 = 0x3432_5258;

    #[derive(Default)]
    struct Seen {
        /// The layouts the compositor takes buffers of this kind in.
        layouts: Vec<u64>,
        configured: bool,
    }

    impl Dispatch<ZwpLinuxDmabufV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &ZwpLinuxDmabufV1,
            event: zwp_linux_dmabuf_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwp_linux_dmabuf_v1::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } = event
            {
                if format == XRGB {
                    seen.layouts
                        .push((u64::from(modifier_hi) << 32) | u64::from(modifier_lo));
                }
            }
        }
    }

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Seen {
        fn event(
            _: &mut Self,
            shell: &xdg_wm_base::XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                shell.pong(serial);
            }
        }
    }

    impl Dispatch<xdg_surface::XdgSurface, ()> for Seen {
        fn event(
            seen: &mut Self,
            shown: &xdg_surface::XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                shown.ack_configure(serial);
                seen.configured = true;
            }
        }
    }

    macro_rules! quiet {
        ($interface:ty, $data:ty) => {
            impl Dispatch<$interface, $data> for Seen {
                fn event(
                    _: &mut Self,
                    _: &$interface,
                    _: <$interface as Proxy>::Event,
                    _: &$data,
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        };
    }
    quiet!(wl_compositor::WlCompositor, ());
    quiet!(wl_surface::WlSurface, ());
    quiet!(wl_buffer::WlBuffer, ());
    quiet!(xdg_toplevel::XdgToplevel, ());
    quiet!(ZwpLinuxBufferParamsV1, ());
    quiet!(wl_registry::WlRegistry, GlobalListContents);

    /// The window, and what the stream agreed on.
    struct Shown {
        queue: EventQueue<Seen>,
        seen: Seen,
        buffers: ZwpLinuxDmabufV1,
        surface: wl_surface::WlSurface,
        /// The size and the layout agreed on.
        agreed: Option<((u32, u32), u64)>,
        shown: bool,
        counted: usize,
    }

    impl Shown {
        /// Take in what the compositor says, without waiting for it.
        fn pump(&mut self) {
            let _ = self.queue.flush();
            if let Some(guard) = self.queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = self.queue.dispatch_pending(&mut self.seen);
            let _ = self.queue.flush();
        }
    }

    fn serialized(value: spa::pod::Value) -> Vec<u8> {
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .map(|(cursor, _)| cursor.into_inner())
            .unwrap_or_default()
    }

    /// Pictures of any size, blue first, in buffers of the card of one
    /// of these layouts and no others.
    fn format(layouts: &[u64]) -> Vec<u8> {
        use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
        use spa::pod::{ChoiceValue, Property, PropertyFlags, Value};
        use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Id, Rectangle};
        let plain = |key: FormatProperties, value: Value| Property {
            key: key.as_raw(),
            flags: PropertyFlags::empty(),
            value,
        };
        let layouts: Vec<i64> = layouts.iter().map(|layout| *layout as i64).collect();
        serialized(Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: vec![
                plain(
                    FormatProperties::MediaType,
                    Value::Id(Id(MediaType::Video.as_raw())),
                ),
                plain(
                    FormatProperties::MediaSubtype,
                    Value::Id(Id(MediaSubtype::Raw.as_raw())),
                ),
                plain(
                    FormatProperties::VideoFormat,
                    Value::Id(Id(spa::param::video::VideoFormat::BGRx.as_raw())),
                ),
                Property {
                    key: FormatProperties::VideoModifier.as_raw(),
                    flags: PropertyFlags::MANDATORY | PropertyFlags::DONT_FIXATE,
                    value: Value::Choice(ChoiceValue::Long(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Enum {
                            default: layouts[0],
                            alternatives: layouts.clone(),
                        },
                    ))),
                },
                plain(
                    FormatProperties::VideoSize,
                    Value::Choice(ChoiceValue::Rectangle(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Range {
                            default: Rectangle {
                                width: 1280,
                                height: 720,
                            },
                            min: Rectangle {
                                width: 1,
                                height: 1,
                            },
                            max: Rectangle {
                                width: 16384,
                                height: 16384,
                            },
                        },
                    ))),
                ),
            ],
        }))
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let node: u32 = std::env::args().nth(1).ok_or("card_taker NODE")?.parse()?;

        // The window first: what the compositor takes is what is asked
        // of the stream.
        let connection = Connection::connect_to_env()?;
        let (globals, mut queue) = registry_queue_init::<Seen>(&connection)?;
        let handle = queue.handle();
        let mut seen = Seen::default();
        let buffers: ZwpLinuxDmabufV1 = globals.bind(&handle, 3..=3, ())?;
        let compositor: wl_compositor::WlCompositor = globals.bind(&handle, 4..=4, ())?;
        let shell: xdg_wm_base::XdgWmBase = globals.bind(&handle, 1..=1, ())?;
        let surface = compositor.create_surface(&handle, ());
        let shown = shell.get_xdg_surface(&surface, &handle, ());
        let window = shown.get_toplevel(&handle, ());
        window.set_title("shared".to_string());
        window.set_app_id("gevrek-card-taker".to_string());
        surface.commit();
        while !seen.configured {
            queue.blocking_dispatch(&mut seen)?;
        }
        queue.roundtrip(&mut seen)?;
        if seen.layouts.is_empty() {
            return Err("the compositor takes no buffers of the graphics card".into());
        }
        println!(
            "{{\"event\": \"offers\", \"layouts\": {}}}",
            seen.layouts.len()
        );
        let offered = format(&seen.layouts);
        let kept = Rc::new(RefCell::new(Shown {
            queue,
            seen,
            buffers,
            surface,
            agreed: None,
            shown: false,
            counted: 0,
        }));

        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let stream = pw::stream::StreamRc::new(
            core,
            "card-taker",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )?;
        let _listener = stream
            .add_local_listener_with_user_data(Rc::clone(&kept))
            .param_changed(|_, kept, id, param| {
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Some(param) = param else {
                    return;
                };
                let mut info = spa::param::video::VideoInfoRaw::new();
                if info.parse(param).is_err() {
                    return;
                }
                let (size, layout) = (info.size(), info.modifier());
                kept.borrow_mut().agreed = Some(((size.width, size.height), layout));
                println!(
                    "{{\"event\": \"agreed\", \"width\": {}, \"height\": {}, \"layout\": \"{layout:#x}\"}}",
                    size.width, size.height
                );
            })
            .process(|stream, kept| {
                let mut kept = kept.borrow_mut();
                // SAFETY: a buffer of this stream (or none); handed back
                // below, but for the one that is shown.
                let taken = unsafe { stream.dequeue_raw_buffer() };
                if taken.is_null() {
                    return;
                }
                // SAFETY: PipeWire's own buffer: its first piece and that
                // piece's chunk are read.
                let (kind, fd, stride, offset, size) = unsafe {
                    let buffer = (*taken).buffer;
                    if buffer.is_null() || (*buffer).n_datas == 0 || (*buffer).datas.is_null() {
                        stream.queue_raw_buffer(taken);
                        return;
                    }
                    let data = &*(*buffer).datas;
                    if data.chunk.is_null() {
                        (data.type_, data.fd, 0, 0, 0)
                    } else {
                        let chunk = &*data.chunk;
                        (data.type_, data.fd, chunk.stride, chunk.offset, chunk.size)
                    }
                };
                let of_card = kind == spa::sys::SPA_DATA_DmaBuf && fd >= 0;
                if size == 0 {
                    // (Nothing was drawn into it.)
                    // SAFETY: taken from this stream just above.
                    unsafe { stream.queue_raw_buffer(taken) };
                    return;
                }
                kept.counted += 1;
                println!(
                    "{{\"event\": \"buffer\", \"of_card\": {of_card}, \"stride\": {stride}, \"count\": {}}}",
                    kept.counted
                );
                let agreed = kept.agreed;
                if let (false, true, Some(((width, height), layout))) = (kept.shown, of_card, agreed) {
                    let handle = kept.queue.handle();
                    let params = kept.buffers.create_params(&handle, ());
                    // SAFETY: the stream's descriptor, open while the
                    // buffer is this program's (it is kept from here on).
                    let fd = unsafe { BorrowedFd::borrow_raw(fd as i32) };
                    params.add(
                        fd,
                        0,
                        offset,
                        stride as u32,
                        (layout >> 32) as u32,
                        layout as u32,
                    );
                    let buffer = params.create_immed(
                        width as i32,
                        height as i32,
                        XRGB,
                        zwp_linux_buffer_params_v1::Flags::empty(),
                        &handle,
                        (),
                    );
                    params.destroy();
                    kept.surface.attach(Some(&buffer), 0, 0);
                    kept.surface.damage_buffer(0, 0, width as i32, height as i32);
                    kept.surface.commit();
                    kept.shown = true;
                    kept.pump();
                    println!("{{\"event\": \"shown\"}}");
                    // (Not handed back: the stream leaves it as it is.)
                    return;
                }
                // SAFETY: taken from this stream just above.
                unsafe { stream.queue_raw_buffer(taken) };
            })
            .register()?;
        let mut params: Vec<&spa::pod::Pod> =
            spa::pod::Pod::from_bytes(&offered).into_iter().collect();
        stream.connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT,
            &mut params,
        )?;
        // What the compositor says of the window is answered meanwhile.
        let pumped = Rc::clone(&kept);
        let timer = mainloop.loop_().add_timer(move |_| {
            if let Ok(mut kept) = pumped.try_borrow_mut() {
                kept.pump();
            }
        });
        let every = std::time::Duration::from_millis(50);
        let _ = timer.update_timer(Some(every), Some(every));
        mainloop.run();
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    taker::run()
}
