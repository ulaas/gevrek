//! What lies behind a see-through surface, soft: the windows too, not
//! the wallpaper alone.
//!
//! A surface whose layer rule says `blur`, a window that is drawn
//! see-through, and the veil over the desktop while it changes to the
//! lock screen show what is behind them out of focus. For that a display
//! is drawn a second time, small and apart from what goes to the screen,
//! without those things themselves and without the pointer; that small
//! picture is softened and kept, and a part of it is drawn behind each of
//! them the next time the display is drawn. It is made again only when
//! what it shows has changed (the small drawing says whether anything in
//! it is another), so a desktop that stands still costs nothing.
//!
//! The picture is one frame behind what it lies under. Until there is
//! one (the first frame a see-through thing is shown) the wallpaper's
//! own soft copy stands in.
//!
//! Where the display is drawn by the graphics card, all of it stays
//! there: the small picture is softened by a program of the card (a few
//! passes of a blur, across and down) and drawn from the card's own
//! memory. The renderer in memory (the checks) reads the small picture
//! and softens it on the processor.

use crate::capture::CaptureTarget;
use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::texture::TextureBuffer;
use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::gles::{
    GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName, UniformType, UniformValue,
};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::{
    Bind, ExportMem, Frame as _, ImportAll, ImportMem, Offscreen, Renderer,
};
use smithay::reexports::pixman;
use smithay::utils::{Rectangle, Transform};
use std::collections::{HashMap, HashSet};

/// How wide the small picture is at the most, in pixels.
const WIDTH: f64 = 480.0;
/// How far a pixel of the display is spread, in the display's own pixels
/// at a scale of one (what the wallpaper's soft copy is made with).
const SPREAD: f64 = 24.0;

/// A soft picture, as its renderer keeps it.
#[derive(Clone)]
pub enum Picture {
    /// In memory (the wallpaper's own soft copy; the renderer in memory).
    Memory(MemoryRenderBuffer),
    /// In the graphics card's memory.
    Card(TextureBuffer<GlesTexture>),
}

struct Scene {
    soft: Picture,
    size: (i32, i32),
}

/// What a display is drawn small into, kept between drawings so that a
/// drawing can say whether anything in it changed.
struct Small<T> {
    target: CaptureTarget<T>,
    tracker: OutputDamageTracker,
    size: (i32, i32),
    /// Drawn into at least once (its picture can be built on).
    drawn: bool,
}

/// The soft pictures of the displays, and which of them are asked for.
#[derive(Default)]
pub struct Soft {
    scenes: HashMap<MonitorHandle, Scene>,
    /// The displays on which something drawn since the last refresh
    /// showed what is behind it.
    wanted: HashSet<MonitorHandle>,
    /// What the small pictures are drawn into (of the renderer's kind).
    small: HashMap<MonitorHandle, Box<dyn std::any::Any>>,
}

/// The size of the small picture of a display of this size.
pub fn small_size(device: (i32, i32)) -> ((i32, i32), f64) {
    let by = (WIDTH / f64::from(device.0.max(1))).min(1.0);
    (
        (
            ((f64::from(device.0) * by).round() as i32).max(1),
            ((f64::from(device.1) * by).round() as i32).max(1),
        ),
        by,
    )
}

/// Soften a small picture (four bytes a pixel, red first): the bytes as
/// a buffer has them (blue first), every pixel solid.
pub fn softened(size: (i32, i32), pixels: Vec<u8>, spread: f32) -> Option<Vec<u8>> {
    let picture = image::RgbaImage::from_raw(size.0 as u32, size.1 as u32, pixels)?;
    let mut bytes = image::imageops::fast_blur(&picture, spread.max(0.5)).into_raw();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Some(bytes)
}

/// A renderer that can soften a small picture it has drawn into `T`.
pub trait Soften<T>: Renderer {
    /// The picture in `image` (of this size), each pixel spread by
    /// `spread` pixels, as something to draw behind other things.
    fn soften(&mut self, image: &mut T, size: (i32, i32), spread: f32) -> anyhow::Result<Picture>;
}

impl Soften<pixman::Image<'static, 'static>> for PixmanRenderer {
    fn soften(
        &mut self,
        image: &mut pixman::Image<'static, 'static>,
        size: (i32, i32),
        spread: f32,
    ) -> anyhow::Result<Picture> {
        let framebuffer = self
            .bind(image)
            .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
        let mapping = self
            .copy_framebuffer(
                &framebuffer,
                Rectangle::from_size(size.into()),
                Fourcc::Abgr8888,
            )
            .map_err(|error| anyhow::anyhow!("read back: {error:?}"))?;
        let pixels = self
            .map_texture(&mapping)
            .map_err(|error| anyhow::anyhow!("map: {error:?}"))?
            .to_vec();
        let bytes = softened(size, pixels, spread)
            .ok_or_else(|| anyhow::anyhow!("the picture is not of its size"))?;
        Ok(Picture::Memory(MemoryRenderBuffer::from_slice(
            &bytes,
            Fourcc::Argb8888,
            size,
            1,
            Transform::Normal,
            None,
        )))
    }
}

/// The card's program that spreads a picture along one line: nine
/// places of the picture weighed as a bell, taken as five (two of them
/// between two pixels each, which the card mixes by itself).
const BLUR: &str = r#"#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// One step along the line, in parts of the picture.
uniform vec2 blur_step;

void main() {
    vec4 color = texture2D(tex, v_coords) * 0.227027;
    color += texture2D(tex, v_coords + blur_step * 1.384615) * 0.316216;
    color += texture2D(tex, v_coords - blur_step * 1.384615) * 0.316216;
    color += texture2D(tex, v_coords + blur_step * 3.230769) * 0.070270;
    color += texture2D(tex, v_coords - blur_step * 3.230769) * 0.070270;
    // (What lies behind things is solid.)
    gl_FragColor = vec4(color.rgb, 1.0) * alpha;
}
"#;

/// Kept with the context of the renderer: the program, or that it could
/// not be made (said once in the log).
struct BlurProgram(Option<GlesTexProgram>);

fn blur_program(renderer: &mut GlesRenderer) -> Option<GlesTexProgram> {
    if let Some(program) = renderer.egl_context().user_data().get::<BlurProgram>() {
        return program.0.clone();
    }
    let program = renderer
        .compile_custom_texture_shader(BLUR, &[UniformName::new("blur_step", UniformType::_2f)])
        .map_err(|error| log::warn!("blur: the card's program was not made: {error:?}"))
        .ok();
    renderer
        .egl_context()
        .user_data()
        .insert_if_missing(|| BlurProgram(program.clone()));
    program
}

/// One pass: `from` drawn into `to`, spread along `step`.
fn pass(
    renderer: &mut GlesRenderer,
    program: &GlesTexProgram,
    from: &GlesTexture,
    to: &mut GlesTexture,
    size: (i32, i32),
    step: (f32, f32),
) -> anyhow::Result<()> {
    let whole = Rectangle::from_size(size.into());
    let mut framebuffer = renderer
        .bind(to)
        .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
    let mut frame = renderer
        .render(&mut framebuffer, size.into(), Transform::Normal)
        .map_err(|error| anyhow::anyhow!("frame: {error:?}"))?;
    frame
        .render_texture_from_to(
            from,
            Rectangle::from_size((f64::from(size.0), f64::from(size.1)).into()),
            whole,
            &[whole],
            &[],
            Transform::Normal,
            1.0,
            Some(program),
            &[Uniform::new("blur_step", UniformValue::_2f(step.0, step.1))],
        )
        .map_err(|error| anyhow::anyhow!("draw: {error:?}"))?;
    let _ = frame
        .finish()
        .map_err(|error| anyhow::anyhow!("finish: {error:?}"))?;
    Ok(())
}

impl Soften<GlesTexture> for GlesRenderer {
    fn soften(
        &mut self,
        image: &mut GlesTexture,
        size: (i32, i32),
        spread: f32,
    ) -> anyhow::Result<Picture> {
        let program =
            blur_program(self).ok_or_else(|| anyhow::anyhow!("the card has no program for it"))?;
        let make = |renderer: &mut GlesRenderer| {
            Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, size.into())
                .map_err(|error| anyhow::anyhow!("a picture in the card's memory: {error:?}"))
        };
        let (mut first, mut second) = (make(self)?, make(self)?);
        // (A pass spreads a pixel by about two steps: twice across and
        // twice down, with a step that makes up the spread.)
        let step = (spread / 3.0).max(0.5);
        let across = (step / size.0.max(1) as f32, 0.0);
        let down = (0.0, step / size.1.max(1) as f32);
        pass(self, &program, image, &mut first, size, across)?;
        pass(self, &program, &first, &mut second, size, down)?;
        pass(self, &program, &second, &mut first, size, across)?;
        pass(self, &program, &first, &mut second, size, down)?;
        Ok(Picture::Card(TextureBuffer::from_texture(
            self,
            second,
            1,
            Transform::Normal,
            None,
        )))
    }
}

impl State {
    /// What stands behind a see-through thing on this display, soft: the
    /// picture of what is there where one was made, else the wallpaper's
    /// own soft copy; with the picture's size. Asking marks the display:
    /// its picture is made (again) after this drawing.
    pub(crate) fn soft_behind(&self, output: MonitorHandle) -> Option<(Picture, (i32, i32))> {
        let mut soft = self.soft.borrow_mut();
        soft.wanted.insert(output);
        soft.scenes
            .get(&output)
            .map(|scene| (scene.soft.clone(), scene.size))
            .or_else(|| {
                self.wallpapers
                    .borrow()
                    .blurred(output)
                    .map(|(picture, size)| (Picture::Memory(picture), size))
            })
    }

    /// Make the soft pictures that were asked for while the displays
    /// were drawn. Called by a backend after it has drawn; `format` is a
    /// format its renderer can draw into. `changed`: the displays that
    /// were drawn anew (`None`: all of them).
    pub fn refresh_soft<R, T>(
        &mut self,
        renderer: &mut R,
        format: Fourcc,
        changed: Option<&[MonitorHandle]>,
    ) where
        R: Renderer
            + ImportAll
            + ImportMem
            + Offscreen<T>
            + Bind<T>
            + Soften<T>
            + crate::clip::ClipRenderer,
        R::TextureId: Send + Clone + 'static,
        R::Error: std::fmt::Debug,
        T: 'static,
    {
        let wanted = std::mem::take(&mut self.soft.borrow_mut().wanted);
        // A display that was drawn with nothing see-through on it keeps
        // no picture (the next such thing begins with the wallpaper's).
        let drawn = |output: &MonitorHandle| changed.is_none_or(|changed| changed.contains(output));
        {
            let mut soft = self.soft.borrow_mut();
            soft.scenes
                .retain(|output, _| wanted.contains(output) || !drawn(output));
            soft.small
                .retain(|output, _| wanted.contains(output) || !drawn(output));
        }
        if wanted.is_empty() {
            return;
        }
        // While the lock screen is what is drawn, the picture stays what
        // it was of the desktop: the lock screen is not what the veil
        // dissolves into.
        if self.locked && self.lock_look().is_none_or(|look| look.covered) {
            return;
        }
        for output in wanted {
            if !drawn(&output) {
                // (Asked for again when it is drawn.)
                self.soft.borrow_mut().wanted.insert(output);
                continue;
            }
            match self.draw_small::<R, T>(renderer, format, output) {
                Ok(Some((soft, size))) => {
                    self.soft
                        .borrow_mut()
                        .scenes
                        .insert(output, Scene { soft, size });
                    // What lies over it is drawn again, over the new one.
                    self.desktop.needs_redraw = true;
                }
                Ok(None) => {}
                Err(error) => log::debug!("the soft picture of a display: {error}"),
            }
        }
    }

    /// Draw a display small, without what is see-through on it and
    /// without the pointer, and soften it: the picture and its size.
    /// `None`: nothing in it is another than the last time.
    fn draw_small<R, T>(
        &mut self,
        renderer: &mut R,
        format: Fourcc,
        output: MonitorHandle,
    ) -> anyhow::Result<Option<(Picture, (i32, i32))>>
    where
        R: Renderer
            + ImportAll
            + ImportMem
            + Offscreen<T>
            + Bind<T>
            + Soften<T>
            + crate::clip::ClipRenderer,
        R::TextureId: Send + Clone + 'static,
        R::Error: std::fmt::Debug,
        T: 'static,
    {
        let Some((width, height, _)) = self.device_size(output) else {
            return Ok(None);
        };
        if width <= 0 || height <= 0 {
            return Ok(None);
        }
        let scale = self
            .desktop
            .outputs
            .iter()
            .find(|record| record.id == output)
            .map_or(1.0, |record| record.scale);
        let (size, by) = small_size((width, height));
        // What it is drawn into, kept from the last time where the size
        // is the same.
        let mut small = self
            .soft
            .borrow_mut()
            .small
            .remove(&output)
            .and_then(|kept| kept.downcast::<Small<T>>().ok())
            .filter(|small| small.size == size)
            .unwrap_or_else(|| {
                Box::new(Small {
                    target: CaptureTarget::default(),
                    tracker: OutputDamageTracker::new(size, 1.0, Transform::Normal),
                    size,
                    drawn: false,
                })
            });
        let (pointer, pass) = (self.hide_pointer, self.backdrop_pass);
        self.hide_pointer = true;
        self.backdrop_pass = true;
        let elements: Vec<_> = self
            .render_elements(renderer, output)
            .into_iter()
            .map(|element| RescaleRenderElement::from_element(element, (0, 0).into(), scale * by))
            .collect();
        self.hide_pointer = pointer;
        self.backdrop_pass = pass;
        let made = (|| {
            let image = small.target.image_of(renderer, format, size)?;
            let another = {
                let mut framebuffer = renderer
                    .bind(image)
                    .map_err(|error| anyhow::anyhow!("bind: {error:?}"))?;
                // (Built on the last drawing: only what changed is
                // drawn, and whether anything did is told.)
                let age = usize::from(small.drawn);
                let result = small
                    .tracker
                    .render_output(
                        renderer,
                        &mut framebuffer,
                        age,
                        &elements,
                        crate::render::BACKGROUND,
                    )
                    .map_err(|error| anyhow::anyhow!("render: {error:?}"))?;
                result.damage.is_some()
            };
            small.drawn = true;
            if !another {
                return Ok(None);
            }
            let image = small.target.image_of(renderer, format, size)?;
            let soft = renderer.soften(image, size, (SPREAD * scale * by) as f32)?;
            Ok(Some((soft, size)))
        })();
        self.soft.borrow_mut().small.insert(output, small);
        made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_small_picture_is_never_wider_than_it_need_be() {
        assert_eq!(small_size((1920, 1080)), ((480, 270), 0.25));
        assert_eq!(small_size((3840, 2160)).0, (480, 270));
        // A display smaller than that is drawn at its own size.
        assert_eq!(small_size((320, 200)), ((320, 200), 1.0));
        assert_eq!(small_size((0, 0)).0, (1, 1));
    }

    #[test]
    fn a_picture_is_softened_and_solid() {
        // Lines one pixel wide, black and white, half see-through.
        let size = (32, 16);
        let mut pixels = Vec::new();
        for _ in 0..size.1 {
            for x in 0..size.0 {
                let light = if x % 2 == 0 { 255 } else { 0 };
                pixels.extend([light, light, light, 128]);
            }
        }
        let soft = softened(size, pixels, 4.0).expect("a picture");
        assert_eq!(soft.len(), (size.0 * size.1 * 4) as usize);
        let middle = &soft[(8 * size.0 as usize + 16) * 4..][..4];
        assert!(
            (100..=155).contains(&middle[0]),
            "one grey where lines were: {middle:?}"
        );
        assert!(soft.chunks(4).all(|pixel| pixel[3] == 255), "solid");
        // Red and blue change places for the buffer.
        let red = softened((2, 2), [200, 10, 20, 255].repeat(4), 1.0).expect("a picture");
        assert!(red[0] < 60 && red[2] > 150, "{:?}", &red[..4]);
        // Bytes that are no picture of that size.
        assert!(softened((4, 4), vec![0; 7], 1.0).is_none());
    }
}
