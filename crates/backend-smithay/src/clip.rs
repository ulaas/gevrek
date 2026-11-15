//! Round corners for what a window draws. The border and the picture around
//! a window are round at the corners; the content of the window is a plain
//! rectangle, so its corners are cut to the same rounding when it is drawn.
//!
//! The cut is made by the texture program: the renderer that draws with
//! OpenGL gets one that fades a pixel out by its distance from a rectangle
//! with round corners. The renderer that draws in memory (checks, CI) has
//! no such program and draws the content uncut.

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesFrame, GlesRenderer, GlesTexProgram, Uniform, UniformName, UniformType, UniformValue,
};
use smithay::backend::renderer::pixman::PixmanRenderer;
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Renderer;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

/// The texture program of the renderer, with the cut added. Where a pixel
/// is on the target comes from `gl_FragCoord`, so the cut is the same
/// whatever the buffer of the surface is like (scaled, cropped, turned).
const SHADER: &str = r#"#version 100

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

// From a pixel of the target to a pixel of the output: the rows of a
// matrix, and what is taken off first.
uniform vec4 clip_basis;
uniform vec2 clip_shift;
// The rectangle the content is cut to: its centre and half its size.
uniform vec4 clip_rect;
uniform float clip_radius;

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    vec2 target = gl_FragCoord.xy - clip_shift;
    vec2 point = vec2(dot(clip_basis.xy, target), dot(clip_basis.zw, target)) - clip_rect.xy;
    vec2 corner = abs(point) - (clip_rect.zw - vec2(clip_radius));
    float distance = length(max(corner, 0.0)) + min(max(corner.x, corner.y), 0.0) - clip_radius;
    color = color * clamp(0.5 - distance, 0.0, 1.0);

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

/// The rectangle a frame cuts to, in the pixels it draws in.
#[derive(Debug, Clone, Copy)]
pub struct Cut {
    pub centre: (f32, f32),
    pub half: (f32, f32),
    pub radius: f32,
}

/// A renderer that may cut what it draws to a rectangle with round corners.
pub trait ClipRenderer: Renderer {
    /// The program that cuts, made the first time it is asked for. `None`
    /// where the renderer has none: the content is then drawn uncut.
    fn clip_program(&mut self) -> Option<GlesTexProgram>;

    /// What the frame draws from here on is cut.
    fn set_cut(frame: &mut Self::Frame<'_, '_>, program: &GlesTexProgram, cut: Cut);

    /// The frame draws uncut again.
    fn clear_cut(frame: &mut Self::Frame<'_, '_>);

    /// A soft picture that is kept in the graphics card's memory
    /// (`soft.rs`), as this renderer draws from it. `None`: it is not
    /// one of this renderer's.
    fn card_picture(
        picture: &crate::soft::Picture,
    ) -> Option<smithay::backend::renderer::element::texture::TextureBuffer<Self::TextureId>>;

    /// A buffer of the graphics card's own memory (one a program handed
    /// over for a picture of the screen) as something to draw into.
    /// `None` where this renderer cannot draw into such a one.
    fn bind_card<'a>(
        &mut self,
        dmabuf: &'a mut smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Option<<Self as smithay::backend::renderer::RendererSuper>::Framebuffer<'a>>;
}

/// Kept with the context of the renderer: the program, or that it could not
/// be made (said once in the log).
struct Program(Option<GlesTexProgram>);

impl ClipRenderer for GlesRenderer {
    fn clip_program(&mut self) -> Option<GlesTexProgram> {
        if let Some(program) = self.egl_context().user_data().get::<Program>() {
            return program.0.clone();
        }
        let program = self
            .compile_custom_texture_shader(
                SHADER,
                &[
                    UniformName::new("clip_basis", UniformType::_4f),
                    UniformName::new("clip_shift", UniformType::_2f),
                    UniformName::new("clip_rect", UniformType::_4f),
                    UniformName::new("clip_radius", UniformType::_1f),
                ],
            )
            .map_err(|error| log::warn!("round corners: the program was not made: {error:?}"))
            .ok();
        self.egl_context()
            .user_data()
            .insert_if_missing(|| Program(program.clone()));
        program
    }

    fn set_cut(frame: &mut GlesFrame<'_, '_>, program: &GlesTexProgram, cut: Cut) {
        let (basis, shift) = target_to_output(frame.projection());
        frame.override_default_tex_program(
            program.clone(),
            vec![
                Uniform::new(
                    "clip_basis",
                    UniformValue::_4f(basis[0], basis[1], basis[2], basis[3]),
                ),
                Uniform::new("clip_shift", UniformValue::_2f(shift.0, shift.1)),
                Uniform::new(
                    "clip_rect",
                    UniformValue::_4f(cut.centre.0, cut.centre.1, cut.half.0, cut.half.1),
                ),
                Uniform::new("clip_radius", UniformValue::_1f(cut.radius)),
            ],
        );
    }

    fn clear_cut(frame: &mut GlesFrame<'_, '_>) {
        frame.clear_tex_program_override();
    }

    fn card_picture(
        picture: &crate::soft::Picture,
    ) -> Option<smithay::backend::renderer::element::texture::TextureBuffer<Self::TextureId>> {
        match picture {
            crate::soft::Picture::Card(buffer) => Some(buffer.clone()),
            crate::soft::Picture::Memory(_) => None,
        }
    }

    fn bind_card<'a>(
        &mut self,
        dmabuf: &'a mut smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Option<<Self as smithay::backend::renderer::RendererSuper>::Framebuffer<'a>> {
        use smithay::backend::renderer::Bind;
        self.bind(dmabuf)
            .map_err(|error| log::debug!("a program's buffer could not be drawn into: {error:?}"))
            .ok()
    }
}

impl ClipRenderer for PixmanRenderer {
    fn clip_program(&mut self) -> Option<GlesTexProgram> {
        None
    }

    fn set_cut(_frame: &mut Self::Frame<'_, '_>, _program: &GlesTexProgram, _cut: Cut) {}

    fn clear_cut(_frame: &mut Self::Frame<'_, '_>) {}

    fn card_picture(
        _: &crate::soft::Picture,
    ) -> Option<smithay::backend::renderer::element::texture::TextureBuffer<Self::TextureId>> {
        None
    }

    fn bind_card<'a>(
        &mut self,
        _: &'a mut smithay::backend::allocator::dmabuf::Dmabuf,
    ) -> Option<<Self as smithay::backend::renderer::RendererSuper>::Framebuffer<'a>> {
        None
    }
}

/// How to get from a pixel of the target (`gl_FragCoord`) back to a pixel
/// of the output, from the projection of the frame (nine numbers, column
/// by column), which goes the other way and ends between -1 and 1.
///
/// The projection only moves, mirrors and turns by quarter turns, and one
/// pixel of the output is one pixel of the target, so the way back is a
/// shift and then a matrix of zeros and ones (plus or minus): the two rows
/// of that matrix, and the shift.
pub fn target_to_output(projection: &[f32; 9]) -> ([f32; 4], (f32, f32)) {
    let (a, b, tx) = (projection[0], projection[3], projection[6]);
    let (c, d, ty) = (projection[1], projection[4], projection[7]);
    // Half the width and half the height of the target.
    let sx = 1.0 / (a.abs() + b.abs()).max(f32::EPSILON);
    let sy = 1.0 / (c.abs() + d.abs()).max(f32::EPSILON);
    (
        [a * sx, c * sy, b * sx, d * sy],
        ((tx + 1.0) * sx, (ty + 1.0) * sy),
    )
}

/// An element, cut to a rectangle with round corners.
#[derive(Debug)]
pub struct Clipped<E> {
    inner: E,
    /// What it is cut to, in the coordinates the element is placed in.
    rect: Rectangle<i32, Physical>,
    radius: f32,
    program: GlesTexProgram,
}

impl<E: Element> Clipped<E> {
    /// `rect` is in the coordinates `inner` is placed in (those of
    /// `inner.geometry(1.0)`); the radius is in the same pixels.
    pub fn new(
        inner: E,
        rect: Rectangle<i32, Physical>,
        radius: f32,
        program: GlesTexProgram,
    ) -> Self {
        let radius = radius
            .min(rect.size.w as f32 / 2.0)
            .min(rect.size.h as f32 / 2.0)
            .max(0.0);
        Self {
            inner,
            rect,
            radius,
            program,
        }
    }
}

impl<E: Element> Element for Clipped<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    /// What is behind the cut corners shows, so they are not opaque: of
    /// each opaque part only what lies in the cross between the corners is
    /// left.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let regions = self.inner.opaque_regions(scale);
        if regions.is_empty() {
            return regions;
        }
        // The regions are counted from the corner of the element.
        let at = self.inner.geometry(scale).loc;
        let mut rect: Rectangle<i32, Physical> = self.rect.to_f64().upscale(scale).to_i32_round();
        rect.loc -= at;
        let r = (f64::from(self.radius) * scale.x.max(scale.y)).ceil() as i32 + 1;
        // A window smaller than its corners are round has no such cross
        // (and a rectangle may not be made with a size below nothing: that
        // ended the compositor when many windows shared a display).
        let part = |x: i32, y: i32, width: i32, height: i32| {
            (width > 0 && height > 0).then(|| Rectangle::new((x, y).into(), (width, height).into()))
        };
        let parts = [
            part(rect.loc.x, rect.loc.y + r, rect.size.w, rect.size.h - 2 * r),
            part(rect.loc.x + r, rect.loc.y, rect.size.w - 2 * r, rect.size.h),
        ];
        regions
            .iter()
            .flat_map(|region| {
                parts
                    .into_iter()
                    .flatten()
                    .filter_map(|part| region.intersection(part))
            })
            .collect()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R, E> RenderElement<R> for Clipped<E>
where
    R: ClipRenderer,
    E: RenderElement<R>,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        // Where the element ends up may be scaled and moved from where it
        // was placed (the scale of the output): the cut goes along.
        let placed = self.inner.geometry(Scale::from(1.0));
        if placed.size.w <= 0 || placed.size.h <= 0 {
            return self.inner.draw(frame, src, dst, damage, opaque_regions);
        }
        let kx = dst.size.w as f32 / placed.size.w as f32;
        let ky = dst.size.h as f32 / placed.size.h as f32;
        let left = dst.loc.x as f32 + (self.rect.loc.x - placed.loc.x) as f32 * kx;
        let top = dst.loc.y as f32 + (self.rect.loc.y - placed.loc.y) as f32 * ky;
        let half = (
            self.rect.size.w as f32 * kx / 2.0,
            self.rect.size.h as f32 * ky / 2.0,
        );
        let cut = Cut {
            centre: (left + half.0, top + half.1),
            half,
            radius: self.radius * kx.min(ky),
        };
        R::set_cut(frame, &self.program, cut);
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        R::clear_cut(frame);
        result
    }

    /// Never handed to the display as it is: that would show it uncut.
    fn underlying_storage(&self, _renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The way back for a projection that takes (x, y) of the output to a
    /// place between -1 and 1.
    fn back(projection: &[f32; 9], target: (f32, f32)) -> (f32, f32) {
        let (basis, shift) = target_to_output(projection);
        let (x, y) = (target.0 - shift.0, target.1 - shift.1);
        (basis[0] * x + basis[1] * y, basis[2] * x + basis[3] * y)
    }

    /// Where the projection puts a pixel of the output on a target of this
    /// size, as `gl_FragCoord` counts.
    fn forth(projection: &[f32; 9], size: (f32, f32), point: (f32, f32)) -> (f32, f32) {
        let x = projection[0] * point.0 + projection[3] * point.1 + projection[6];
        let y = projection[1] * point.0 + projection[4] * point.1 + projection[7];
        ((x + 1.0) / 2.0 * size.0, (y + 1.0) / 2.0 * size.1)
    }

    #[test]
    fn a_pixel_of_the_target_is_found_on_the_output_again() {
        let (w, h) = (1920.0f32, 1080.0f32);
        // Upright; upside down (a window of the host); a quarter turn, on a
        // target that is as wide as the output is high.
        let projections: [([f32; 9], (f32, f32)); 3] = [
            (
                [2.0 / w, 0.0, 0.0, 0.0, 2.0 / h, 0.0, -1.0, -1.0, 1.0],
                (w, h),
            ),
            (
                [2.0 / w, 0.0, 0.0, 0.0, -2.0 / h, 0.0, -1.0, 1.0, 1.0],
                (w, h),
            ),
            (
                [0.0, 2.0 / w, 0.0, -2.0 / h, 0.0, 0.0, 1.0, -1.0, 1.0],
                (h, w),
            ),
        ];
        for (projection, size) in projections {
            for point in [(0.5, 0.5), (300.5, 20.5), (1919.5, 1079.5)] {
                let found = back(&projection, forth(&projection, size, point));
                assert!(
                    (found.0 - point.0).abs() < 0.01 && (found.1 - point.1).abs() < 0.01,
                    "{point:?} came back as {found:?}"
                );
            }
        }
    }
}
