//! Screen sharing without a copy: the pictures in buffers of the
//! graphics card's own memory.
//!
//! The ordinary way (`screencast.rs`) has the compositor copy each
//! picture into shared memory and the shell copy it on into PipeWire's.
//! Here the stream's buffers are made by the shell in the graphics
//! card's memory; the compositor draws the display straight into the
//! one that is free, and the program that takes the stream is handed
//! that same buffer. Nothing is copied on the way.
//!
//! It is used only where the configuration asks for it
//! (`desktop.platform.wayland.share_card_buffers`) and the compositor
//! offers such pictures, for displays only; whatever cannot be had this
//! way is shared the ordinary way. A program that takes the stream must
//! take buffers of the card in the layout they are made in: one that
//! cannot gets no pictures, which is why this is not the way unless
//! asked for.

use super::{pump, wayland, Cast, Following, Frame, ToStream, What, MOST_A_SECOND, PATIENCE};
use anyhow::Context as _;
use pipewire as pw;
use pw::spa;
use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_client::protocol::wl_buffer;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::Options;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1;

/// `XRGB8888`, as the graphics card's buffers name it.
pub(super) const XRGB: u32 = 0x3432_5258;

/// What the pictures' thread says before the first one: how large they
/// are, the graphics card (the number of its device) and the layouts its
/// buffers may have; or why it cannot be done.
type Described = Result<((u32, u32), u64, Vec<u64>), String>;

/// What the stream's thread tells the pictures' thread.
pub(super) enum FromStream {
    /// A buffer was made: its number, and what a picture needs to be
    /// drawn into it.
    Made {
        key: u64,
        fd: OwnedFd,
        size: (u32, u32),
        stride: u32,
        offset: u32,
        layout: u64,
    },
    /// A buffer is gone.
    Gone(u64),
    /// The buffer to draw the next picture into; none is free (or
    /// nobody takes the stream).
    Use(Option<u64>),
}

/// Begin to share a display this way. An error where it cannot be done
/// so (the caller shares it the ordinary way then).
pub(super) fn start(
    what: &What,
    with_pointer: bool,
    beside: bool,
    ended: impl FnOnce() + Send + 'static,
) -> anyhow::Result<Cast> {
    anyhow::ensure!(
        matches!(what, What::Output(_)),
        "only displays are shared this way"
    );
    let stop = Arc::new(AtomicBool::new(false));
    // Sharing this way began: only then is its end told of (what does
    // not begin is shared the ordinary way, and has not ended).
    let began = Arc::new(AtomicBool::new(false));
    let frame = Arc::new(Mutex::new(Frame::default()));
    let (to_stream, from_capture) = pw::channel::channel::<ToStream>();
    let (to_capture, from_stream) = mpsc::channel::<FromStream>();
    // The pictures: the thread says how large they are and what the
    // card's buffers may be like, or why it cannot be done.
    let (described, description) = mpsc::channel::<Described>();
    {
        let (what, stop, began, frame, to_stream) = (
            what.clone(),
            Arc::clone(&stop),
            Arc::clone(&began),
            Arc::clone(&frame),
            to_stream.clone(),
        );
        std::thread::Builder::new()
            .name("gevrek-share-pictures".to_string())
            .spawn(move || {
                let told = described.clone();
                let done = capture(
                    &what,
                    with_pointer,
                    beside,
                    &stop,
                    &frame,
                    &to_stream,
                    from_stream,
                    told,
                );
                if let Err(error) = done {
                    let _ = described.send(Err(format!("{error:#}")));
                }
                let by_itself = !stop.swap(true, Ordering::Relaxed);
                let _ = to_stream.send(ToStream::Stop);
                if by_itself && began.load(Ordering::Relaxed) {
                    ended();
                }
            })
            .context("no thread for the pictures")?;
    }
    let give_up = |stop: &AtomicBool| stop.store(true, Ordering::Relaxed);
    let (size, device, layouts) = match description.recv_timeout(PATIENCE) {
        Ok(Ok(told)) => told,
        Ok(Err(why)) => {
            give_up(&stop);
            anyhow::bail!(why);
        }
        Err(_) => {
            give_up(&stop);
            anyhow::bail!("the compositor gave no picture");
        }
    };
    let (numbered, node) = mpsc::channel::<Result<u32, String>>();
    {
        let (stop, frame) = (Arc::clone(&stop), Arc::clone(&frame));
        std::thread::Builder::new()
            .name("gevrek-share-stream".to_string())
            .spawn(move || {
                let told = numbered.clone();
                let done = stream(
                    size,
                    device,
                    &layouts,
                    beside,
                    frame,
                    from_capture,
                    to_capture,
                    told,
                );
                if let Err(error) = done {
                    let _ = numbered.send(Err(format!("{error:#}")));
                }
                stop.store(true, Ordering::Relaxed);
            })
            .context("no thread for the stream")?;
    }
    let node = match node.recv_timeout(PATIENCE) {
        Ok(Ok(node)) => node,
        Ok(Err(why)) => {
            give_up(&stop);
            let _ = to_stream.send(ToStream::Stop);
            anyhow::bail!(why);
        }
        Err(_) => {
            give_up(&stop);
            let _ = to_stream.send(ToStream::Stop);
            anyhow::bail!("PipeWire did not take the stream");
        }
    };
    began.store(true, Ordering::Relaxed);
    // (It ended in the moment between: not begun after all.)
    anyhow::ensure!(
        !stop.load(Ordering::Relaxed),
        "the pictures ended as they began"
    );
    Ok(Cast {
        node,
        size,
        stop,
        to_stream,
    })
}

/// The pictures' thread: the display drawn into whichever buffer the
/// stream says is free.
#[allow(clippy::too_many_arguments)]
fn capture(
    what: &What,
    with_pointer: bool,
    beside: bool,
    stop: &AtomicBool,
    frame: &Mutex<Frame>,
    to_stream: &pw::channel::Sender<ToStream>,
    from_stream: mpsc::Receiver<FromStream>,
    described: mpsc::Sender<Described>,
) -> anyhow::Result<()> {
    let mut wayland = wayland()?;
    let handle = wayland.queue.handle();
    let buffers = wayland
        .buffers
        .clone()
        .context("the compositor takes no buffers of the graphics card")?;
    let What::Output(name) = what else {
        anyhow::bail!("only displays are shared this way");
    };
    let output = wayland
        .taking
        .outputs
        .iter()
        .find(|(_, known)| known.as_deref() == Some(name.as_str()))
        .map(|(output, _)| output.clone())
        .with_context(|| format!("there is no display {name}"))?;
    let source = wayland.of_outputs.create_source(&output, &handle, ());
    let options = if with_pointer {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let session = wayland
        .manager
        .create_session(&source, options, &handle, ());
    let began = Instant::now();
    while !wayland.taking.described && !wayland.taking.stopped {
        anyhow::ensure!(
            began.elapsed() < PATIENCE,
            "the compositor did not say how large the picture is"
        );
        pump(&mut wayland, Duration::from_millis(100))?;
    }
    let mut size = wayland
        .taking
        .size
        .filter(|(width, height)| *width > 0 && *height > 0)
        .context("the picture has no size")?;
    let device = wayland
        .taking
        .card
        .context("no buffers of the graphics card are offered for pictures")?;
    anyhow::ensure!(
        !wayland.taking.card_layouts.is_empty(),
        "no layout of the card's buffers is offered"
    );
    let _ = described.send(Ok((size, device, wayland.taking.card_layouts.clone())));
    let mut following = beside
        .then(|| Following::begin(&wayland, &source))
        .flatten();
    // The stream's buffers, as the compositor knows them.
    let mut made: HashMap<u64, wl_buffer::WlBuffer> = HashMap::new();
    let take =
        |told: FromStream, made: &mut HashMap<u64, wl_buffer::WlBuffer>| -> Option<Option<u64>> {
            match told {
                FromStream::Made {
                    key,
                    fd,
                    size,
                    stride,
                    offset,
                    layout,
                } => {
                    let params = buffers.create_params(&handle, ());
                    params.add(
                        fd.as_fd(),
                        0,
                        offset,
                        stride,
                        (layout >> 32) as u32,
                        layout as u32,
                    );
                    let buffer = params.create_immed(
                        size.0 as i32,
                        size.1 as i32,
                        XRGB,
                        zwp_linux_buffer_params_v1::Flags::empty(),
                        &handle,
                        (),
                    );
                    params.destroy();
                    if let Some(old) = made.insert(key, buffer) {
                        old.destroy();
                    }
                    None
                }
                FromStream::Gone(key) => {
                    if let Some(old) = made.remove(&key) {
                        old.destroy();
                    }
                    None
                }
                FromStream::Use(key) => Some(key),
            }
        };
    let mut last = Instant::now() - Duration::from_secs(1);
    while !stop.load(Ordering::Relaxed) && !wayland.taking.stopped {
        // Another size from now on: the stream makes its buffers anew.
        if let Some(now) = wayland.taking.size.filter(|now| *now != size && now.0 > 0) {
            size = now;
            let _ = to_stream.send(ToStream::Size(size.0, size.1));
        }
        let rest = super::pause(last.elapsed(), MOST_A_SECOND);
        if !rest.is_zero() {
            std::thread::sleep(rest);
        }
        // The buffer the next picture goes into.
        let _ = to_stream.send(ToStream::Want);
        let mut key = None;
        let asked = Instant::now();
        while asked.elapsed() < Duration::from_millis(400) {
            match from_stream.recv_timeout(Duration::from_millis(100)) {
                Ok(told) => {
                    if let Some(answer) = take(told, &mut made) {
                        key = answer;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
        let Some(key) = key else {
            // (Nobody takes the stream, or all its buffers are out.)
            pump(&mut wayland, Duration::from_millis(60))?;
            continue;
        };
        let Some(buffer) = made.get(&key).cloned() else {
            let _ = to_stream.send(ToStream::Unused(key));
            continue;
        };
        let asked = session.create_frame(&handle, ());
        asked.attach_buffer(&buffer);
        asked.damage_buffer(0, 0, size.0 as i32, size.1 as i32);
        asked.capture();
        wayland.taking.frame = None;
        while wayland.taking.frame.is_none()
            && !wayland.taking.stopped
            && !stop.load(Ordering::Relaxed)
        {
            pump(&mut wayland, Duration::from_millis(100))?;
            if let Some(following) = following.as_mut() {
                following.step(&mut wayland, frame, to_stream);
            }
            while let Ok(told) = from_stream.try_recv() {
                let _ = take(told, &mut made);
            }
        }
        asked.destroy();
        let whole = wayland.taking.frame == Some(true) && made.contains_key(&key);
        if whole {
            last = Instant::now();
            let _ = to_stream.send(ToStream::Filled(key));
        } else {
            let _ = to_stream.send(ToStream::Unused(key));
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    if let Some(following) = following {
        following.end();
    }
    for (_, buffer) in made {
        buffer.destroy();
    }
    session.destroy();
    source.destroy();
    let _ = wayland.queue.roundtrip(&mut wayland.taking);
    Ok(())
}

/// What the stream offers: pictures of this size in buffers of the card
/// of this layout, blue first.
fn format(width: u32, height: u32, layout: u64) -> Vec<u8> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::pod::{ChoiceValue, Property, PropertyFlags, Value};
    use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle};
    let plain = |key: FormatProperties, value: Value| Property {
        key: key.as_raw(),
        flags: PropertyFlags::empty(),
        value,
    };
    let most = Fraction {
        num: MOST_A_SECOND,
        denom: 1,
    };
    let object = spa::pod::Object {
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
            // (The layout is a must: these are no buffers to read as
            // plain memory.)
            Property {
                key: FormatProperties::VideoModifier.as_raw(),
                flags: PropertyFlags::MANDATORY,
                value: Value::Long(layout as i64),
            },
            plain(
                FormatProperties::VideoSize,
                Value::Rectangle(Rectangle { width, height }),
            ),
            plain(
                FormatProperties::VideoFramerate,
                Value::Fraction(Fraction { num: 0, denom: 1 }),
            ),
            plain(
                FormatProperties::VideoMaxFramerate,
                Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: most,
                        min: Fraction { num: 1, denom: 1 },
                        max: most,
                    },
                ))),
            ),
        ],
    };
    super::serialized(Value::Object(object))
}

/// The stream's buffers: one piece each, of the card's memory.
fn buffers() -> Vec<u8> {
    use spa::pod::{ChoiceValue, Property, PropertyFlags, Value};
    use spa::utils::{Choice, ChoiceEnum, ChoiceFlags};
    let kinds = 1 << spa::sys::SPA_DATA_DmaBuf;
    let object = spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_buffers,
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 4,
                        min: 2,
                        max: 8,
                    },
                ))),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_blocks,
                flags: PropertyFlags::empty(),
                value: Value::Int(1),
            },
            Property {
                key: spa::sys::SPA_PARAM_BUFFERS_dataType,
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: kinds,
                        flags: vec![kinds],
                    },
                ))),
            },
        ],
    };
    super::serialized(Value::Object(object))
}

/// The graphics card, by the number the system has for its device.
fn card(device: u64) -> anyhow::Result<gbm::Device<std::fs::File>> {
    use std::os::unix::fs::MetadataExt;
    let node = std::fs::read_dir("/dev/dri")?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| std::fs::metadata(path).is_ok_and(|about| about.rdev() == device))
        .context("the graphics card's device is not found")?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
        .with_context(|| format!("{}", node.display()))?;
    gbm::Device::new(file).context("the graphics card makes no buffers")
}

/// A buffer of the card for a picture of this size.
fn buffer_of(
    card: &gbm::Device<std::fs::File>,
    size: (u32, u32),
    layouts: &[u64],
) -> anyhow::Result<gbm::BufferObject<()>> {
    card.create_buffer_object_with_modifiers2::<()>(
        size.0,
        size.1,
        gbm::Format::Xrgb8888,
        layouts.iter().map(|layout| gbm::Modifier::from(*layout)),
        gbm::BufferObjectFlags::RENDERING,
    )
    .or_else(|_| {
        card.create_buffer_object::<()>(
            size.0,
            size.1,
            gbm::Format::Xrgb8888,
            gbm::BufferObjectFlags::RENDERING,
        )
    })
    .context("the graphics card made no buffer")
}

/// What the stream's thread keeps.
struct Kept {
    card: gbm::Device<std::fs::File>,
    /// The one layout the buffers are made in.
    layout: u64,
    size: (u32, u32),
    /// The buffers that are there, by PipeWire's own (as a number):
    /// the card's buffer, kept while PipeWire has it, and how its rows
    /// lie.
    made: HashMap<u64, (gbm::BufferObject<()>, OwnedFd, u32, u32)>,
    to_capture: mpsc::Sender<FromStream>,
    frame: Arc<Mutex<Frame>>,
    beside: bool,
    numbered: Option<mpsc::Sender<Result<u32, String>>>,
}

/// The stream's thread.
#[allow(clippy::too_many_arguments)]
fn stream(
    size: (u32, u32),
    device: u64,
    layouts: &[u64],
    beside: bool,
    frame: Arc<Mutex<Frame>>,
    from_capture: pw::channel::Receiver<ToStream>,
    to_capture: mpsc::Sender<FromStream>,
    numbered: mpsc::Sender<Result<u32, String>>,
) -> anyhow::Result<()> {
    use pw::stream::{StreamFlags, StreamState};
    let card = card(device)?;
    // The layout the card makes them in (one is made to find out).
    let layout = u64::from(buffer_of(&card, size, layouts)?.modifier());
    log::info!(
        "Sharing in buffers of the graphics card: {}x{}, layout {layout:#x}",
        size.0,
        size.1
    );
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "gevrek-screencast",
        pw::properties::properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::NODE_NAME => "gevrek-screencast",
            *pw::keys::NODE_DESCRIPTION => "gevrek screen sharing",
        },
    )?;
    let kept = Rc::new(std::cell::RefCell::new(Kept {
        card,
        layout,
        size,
        made: HashMap::new(),
        to_capture,
        frame,
        beside,
        numbered: Some(numbered),
    }));
    let _listener = stream
        .add_local_listener_with_user_data(Rc::clone(&kept))
        .state_changed(|stream, kept, _, new| match new {
            StreamState::Paused | StreamState::Streaming => {
                if let Some(numbered) = kept.borrow_mut().numbered.take() {
                    let _ = numbered.send(Ok(stream.node_id()));
                }
            }
            StreamState::Error(why) => {
                if let Some(numbered) = kept.borrow_mut().numbered.take() {
                    let _ = numbered.send(Err(why));
                }
            }
            _ => {}
        })
        .param_changed(|stream, kept, id, param| {
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
            let agreed = info.size();
            kept.borrow_mut().size = (agreed.width, agreed.height);
            let (wanted, room) = (buffers(), super::pointer_param());
            let mut params: Vec<&spa::pod::Pod> =
                spa::pod::Pod::from_bytes(&wanted).into_iter().collect();
            if kept.borrow().beside {
                params.extend(spa::pod::Pod::from_bytes(&room));
            }
            let _ = stream.update_params(&mut params);
        })
        .add_buffer(|_, kept, given| {
            let mut kept = kept.borrow_mut();
            let (size, layout) = (kept.size, kept.layout);
            let made = buffer_of(&kept.card, size, &[layout]).and_then(|buffer| {
                let fd = buffer.fd().context("the buffer cannot be handed over")?;
                Ok((buffer, fd))
            });
            let (buffer, fd) = match made {
                Ok(made) => made,
                Err(error) => {
                    log::warn!("a buffer for the stream: {error:#}");
                    return;
                }
            };
            let (stride, offset) = (buffer.stride(), buffer.offset(0));
            // SAFETY: PipeWire's own buffer, being set up: its first
            // piece is told where the card's memory is (a descriptor
            // that is kept open for as long as the buffer is there).
            unsafe {
                let inner = (*given).buffer;
                if inner.is_null() || (*inner).n_datas == 0 || (*inner).datas.is_null() {
                    return;
                }
                let data = &mut *(*inner).datas;
                data.type_ = spa::sys::SPA_DATA_DmaBuf;
                data.flags = spa::sys::SPA_DATA_FLAG_READWRITE;
                data.fd = i64::from(fd.as_raw_fd());
                data.mapoffset = 0;
                data.maxsize = stride * size.1;
                data.data = std::ptr::null_mut();
                if !data.chunk.is_null() {
                    (*data.chunk).offset = offset;
                    (*data.chunk).stride = stride as i32;
                    (*data.chunk).size = stride * size.1;
                }
            }
            let key = given as u64;
            if let Ok(handed) = fd.try_clone() {
                let _ = kept.to_capture.send(FromStream::Made {
                    key,
                    fd: handed,
                    size,
                    stride,
                    offset,
                    layout,
                });
                kept.made.insert(key, (buffer, fd, stride, offset));
            }
        })
        .remove_buffer(|_, kept, given| {
            let mut kept = kept.borrow_mut();
            let key = given as u64;
            if kept.made.remove(&key).is_some() {
                let _ = kept.to_capture.send(FromStream::Gone(key));
            }
        })
        .process(|_, _| {
            // (The buffers go out as they are filled: `Filled` below.)
        })
        .register()?;
    let offered = format(size.0, size.1, layout);
    let mut params: Vec<&spa::pod::Pod> = spa::pod::Pod::from_bytes(&offered).into_iter().collect();
    stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::DRIVER | StreamFlags::ALLOC_BUFFERS,
        &mut params,
    )?;
    let (told, quit, shared) = (stream.clone(), mainloop.clone(), Rc::clone(&kept));
    let _attached = from_capture.attach(mainloop.loop_(), move |message| {
        let kept = shared.borrow_mut();
        match message {
            // The pictures' thread asks for a buffer to draw into.
            ToStream::Want => {
                let mut free = None;
                if told.state() == StreamState::Streaming {
                    // SAFETY: a buffer of this stream (or none); kept
                    // out until it is `Filled` or `Unused`.
                    let taken = unsafe { told.dequeue_raw_buffer() };
                    if !taken.is_null() {
                        if kept.made.contains_key(&(taken as u64)) {
                            free = Some(taken as u64);
                        } else {
                            // SAFETY: just taken from this stream.
                            unsafe { told.queue_raw_buffer(taken) };
                        }
                    }
                }
                let _ = kept.to_capture.send(FromStream::Use(free));
            }
            ToStream::Filled(key) | ToStream::Unused(key) => {
                let whole = matches!(message, ToStream::Filled(_));
                let Some((_, _, stride, offset)) = kept.made.get(&key) else {
                    return;
                };
                let (stride, offset, height) = (*stride, *offset, kept.size.1);
                let given = key as *mut pw::sys::pw_buffer;
                // SAFETY: the buffer is one this stream made and has not
                // taken back (it is in `made`); it was taken out by
                // `Want` and goes back now.
                unsafe {
                    let inner = (*given).buffer;
                    if !inner.is_null() && (*inner).n_datas > 0 && !(*inner).datas.is_null() {
                        if let (true, true) = (whole, kept.beside) {
                            let frame = kept
                                .frame
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner());
                            if let Some(seen) = frame.pointer.as_ref() {
                                super::write_pointer(inner, seen);
                            }
                        }
                        let data = &mut *(*inner).datas;
                        if !data.chunk.is_null() {
                            (*data.chunk).offset = offset;
                            (*data.chunk).stride = stride as i32;
                            (*data.chunk).size = if whole { stride * height } else { 0 };
                        }
                    }
                    told.queue_raw_buffer(given);
                }
                if whole {
                    let _ = told.trigger_process();
                }
            }
            ToStream::Size(width, height) => {
                let offered = format(width, height, kept.layout);
                if let Some(pod) = spa::pod::Pod::from_bytes(&offered) {
                    let _ = told.update_params(&mut [pod]);
                }
            }
            // (The pointer alone moved: it goes with the next picture.)
            ToStream::Frame => {}
            ToStream::Stop => quit.quit(),
        }
    });
    mainloop.run();
    Ok(())
}
