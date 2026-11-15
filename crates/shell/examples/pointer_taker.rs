//! A program that takes a shared stream and reads the pointer that
//! comes beside its pictures: for the checks of screen sharing.
//!
//!     pointer_taker NODE [HOW_MANY]
//!
//! It takes the stream of that node in PipeWire, asks for room for the
//! pointer beside each picture, and says of each buffer that has one
//! where the pointer is, its hot spot, and how large and how solid its
//! own picture is; after so many it ends (three unless said).

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pipewire as pw;
    use pw::spa;
    use spa::pod::{Property, PropertyFlags, Value};
    use spa::utils::{Choice, ChoiceEnum, ChoiceFlags};
    use std::cell::Cell;
    use std::rc::Rc;

    let mut arguments = std::env::args().skip(1);
    let node: u32 = arguments
        .next()
        .ok_or("pointer_taker NODE [HOW_MANY]")?
        .parse()?;
    let wanted: usize = arguments.next().map_or(Ok(3), |text| text.parse())?;

    fn serialized(value: Value) -> Vec<u8> {
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &value)
            .map(|(cursor, _)| cursor.into_inner())
            .unwrap_or_default()
    }
    let room = |side: usize| {
        (std::mem::size_of::<spa::sys::spa_meta_cursor>()
            + std::mem::size_of::<spa::sys::spa_meta_bitmap>()
            + side * side * 4) as i32
    };
    // Room for the pointer beside each picture: whatever the stream has.
    let pointer = serialized(Value::Object(spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamMeta,
        id: spa::sys::SPA_PARAM_Meta,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_META_type,
                flags: PropertyFlags::empty(),
                value: Value::Id(spa::utils::Id(spa::sys::SPA_META_Cursor)),
            },
            Property {
                key: spa::sys::SPA_PARAM_META_size,
                flags: PropertyFlags::empty(),
                value: Value::Choice(spa::pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: room(64),
                        min: room(1),
                        max: room(1024),
                    },
                ))),
            },
        ],
    }));
    // Pictures of any size, blue first.
    let format = {
        use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
        use spa::utils::Rectangle;
        serialized(Value::Object(spa::pod::object!(
            spa::utils::SpaTypes::ObjectParamFormat,
            spa::param::ParamType::EnumFormat,
            spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
            spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
            spa::pod::property!(
                FormatProperties::VideoFormat,
                Id,
                spa::param::video::VideoFormat::BGRx
            ),
            spa::pod::property!(
                FormatProperties::VideoSize,
                Choice,
                Range,
                Rectangle,
                Rectangle {
                    width: 1280,
                    height: 720
                },
                Rectangle {
                    width: 1,
                    height: 1
                },
                Rectangle {
                    width: 16384,
                    height: 16384
                }
            ),
        )))
    };

    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "pointer-taker",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let seen = Rc::new(Cell::new(0usize));
    let (counted, quit, asked) = (Rc::clone(&seen), mainloop.clone(), pointer.clone());
    let _listener = stream
        .add_local_listener_with_user_data(())
        .param_changed(move |stream, (), id, param| {
            if id != spa::param::ParamType::Format.as_raw() || param.is_none() {
                return;
            }
            if let Some(pod) = spa::pod::Pod::from_bytes(&asked) {
                let _ = stream.update_params(&mut [pod]);
            }
        })
        .process(move |stream, ()| {
            // SAFETY: a buffer of this stream (or none), handed back below.
            let taken = unsafe { stream.dequeue_raw_buffer() };
            if taken.is_null() {
                return;
            }
            // SAFETY: PipeWire's own buffer and what lies beside its
            // picture, read within the sizes PipeWire and the structures
            // themselves give.
            unsafe {
                let buffer = (*taken).buffer;
                let head = std::mem::size_of::<spa::sys::spa_meta_cursor>();
                let meta = spa::sys::spa_buffer_find_meta(buffer, spa::sys::SPA_META_Cursor);
                if !meta.is_null() && (*meta).size as usize >= head && !(*meta).data.is_null() {
                    let whole = (*meta).size as usize;
                    let pointer = (*meta).data as *const spa::sys::spa_meta_cursor;
                    let mut picture = (0u32, 0u32, 0usize);
                    let offset = (*pointer).bitmap_offset as usize;
                    let bitmap_head = std::mem::size_of::<spa::sys::spa_meta_bitmap>();
                    if (*pointer).id != 0 && offset >= head && offset + bitmap_head <= whole {
                        let bitmap =
                            (pointer as *const u8).add(offset) as *const spa::sys::spa_meta_bitmap;
                        let (width, height) = ((*bitmap).size.width, (*bitmap).size.height);
                        let from = (*bitmap).offset as usize;
                        let bytes = (width * height * 4) as usize;
                        if from >= bitmap_head && offset + from + bytes <= whole {
                            let pixels =
                                std::slice::from_raw_parts((bitmap as *const u8).add(from), bytes);
                            let solid = pixels.chunks(4).filter(|pixel| pixel[3] > 200).count();
                            picture = (width, height, solid);
                        }
                    }
                    println!(
                        "{{\"id\": {}, \"x\": {}, \"y\": {}, \"hot_x\": {}, \"hot_y\": {}, \"width\": {}, \"height\": {}, \"solid\": {}}}",
                        (*pointer).id,
                        (*pointer).position.x,
                        (*pointer).position.y,
                        (*pointer).hotspot.x,
                        (*pointer).hotspot.y,
                        picture.0,
                        picture.1,
                        picture.2
                    );
                    counted.set(counted.get() + 1);
                }
                stream.queue_raw_buffer(taken);
            }
            if counted.get() >= wanted {
                quit.quit();
            }
        })
        .register()?;
    let mut params: Vec<&spa::pod::Pod> = spa::pod::Pod::from_bytes(&format).into_iter().collect();
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    mainloop.run();
    Ok(())
}
