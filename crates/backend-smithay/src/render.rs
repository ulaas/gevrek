//! What is on an output, as render elements: the windows the desktop shows,
//! their popups, and the borders gevrek draws around them.

use crate::clip::{ClipRenderer, Clipped};
use crate::state::State;
use gevrek_engine::outputs::MonitorHandle;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::surface::{
    render_elements_from_surface_tree, WaylandSurfaceRenderElement,
};
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::utils::{CropRenderElement, RescaleRenderElement};
use smithay::backend::renderer::element::{render_elements, Kind};
use smithay::backend::renderer::{Color32F, ImportAll, ImportMem, Renderer, RendererSuper};
use smithay::desktop::{layer_map_for_output, PopupKind, PopupManager};
use smithay::input::pointer::{CursorImageStatus, CursorImageSurfaceData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Physical, Point, Rectangle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::compositor::{
    with_surface_tree_downward, SurfaceAttributes, TraversalAction,
};
use smithay::wayland::shell::wlr_layer::Layer;

render_elements! {
    pub OutputElement<R> where R: ImportAll + ImportMem + ClipRenderer;
    Surface=WaylandSurfaceRenderElement<R>,
    Scaled=RescaleRenderElement<WaylandSurfaceRenderElement<R>>,
    // The content of a window with round corners (clip.rs).
    Clipped=Clipped<WaylandSurfaceRenderElement<R>>,
    ClippedScaled=Clipped<RescaleRenderElement<WaylandSurfaceRenderElement<R>>>,
    // The content of a tiled window that is larger than its tile.
    Cropped=CropRenderElement<WaylandSurfaceRenderElement<R>>,
    Solid=SolidColorRenderElement,
    Memory=MemoryRenderBufferRenderElement<R>,
    // A strip of tabs, round at the corners its tile is round at.
    ClippedMemory=Clipped<MemoryRenderBufferRenderElement<R>>,
    // What lies behind a see-through thing, soft, from the graphics
    // card's own memory (soft.rs).
    Texture=TextureRenderElement<<R as RendererSuper>::TextureId>,
    ClippedTexture=Clipped<TextureRenderElement<<R as RendererSuper>::TextureId>>,
}

/// The colour behind the windows.
pub const BACKGROUND: Color32F = Color32F::new(0.07, 0.07, 0.09, 1.0);

/// The dark at the edges of the change between desktop and lock screen:
/// a small picture, clear in the middle and dark towards the corners,
/// that is drawn over the whole display.
pub fn vignette() -> MemoryRenderBuffer {
    const SIDE: i32 = 96;
    let mut bytes = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (dx, dy) = (
                (x as f32 + 0.5) / SIDE as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / SIDE as f32 * 2.0 - 1.0,
            );
            // 0 in the middle, 1 at the middle of an edge, more in a corner.
            let far = ((dx * dx + dy * dy).sqrt() - 0.35).clamp(0.0, 1.0) / 1.0;
            let dark = (far * far * (3.0 - 2.0 * far) * 0.85 * 255.0).round() as u8;
            // Black, as dark as that (colour times its own solidity: 0).
            bytes.extend_from_slice(&[0, 0, 0, dark]);
        }
    }
    MemoryRenderBuffer::from_slice(
        &bytes,
        smithay::backend::allocator::Fourcc::Argb8888,
        (SIDE, SIDE),
        1,
        smithay::utils::Transform::Normal,
        None,
    )
}

/// A part of a soft picture as something to draw: from memory, or from
/// the graphics card's own memory where the picture is kept there; cut
/// round at the corners where `cut` says so. `None` where it cannot be
/// drawn by this renderer.
pub(crate) fn soft_element<R>(
    renderer: &mut R,
    picture: &crate::soft::Picture,
    at: (f64, f64),
    alpha: Option<f32>,
    part: Rectangle<f64, smithay::utils::Logical>,
    size: (i32, i32),
    cut: Option<(
        Rectangle<i32, Physical>,
        f32,
        smithay::backend::renderer::gles::GlesTexProgram,
    )>,
) -> Option<OutputElement<R>>
where
    R: Renderer + ImportAll + ImportMem + ClipRenderer,
    R::TextureId: Send + Clone + 'static,
{
    match picture {
        crate::soft::Picture::Memory(buffer) => {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                at,
                buffer,
                alpha,
                Some(part),
                Some(size.into()),
                Kind::Unspecified,
            ) {
                Ok(element) => Some(match cut {
                    Some((rect, radius, program)) => {
                        OutputElement::ClippedMemory(Clipped::new(element, rect, radius, program))
                    }
                    None => OutputElement::Memory(element),
                }),
                Err(error) => {
                    log::debug!("a soft picture: {error:?}");
                    None
                }
            }
        }
        card => {
            let buffer = R::card_picture(card)?;
            let element = TextureRenderElement::from_texture_buffer(
                at,
                &buffer,
                alpha,
                Some(part),
                Some(size.into()),
                Kind::Unspecified,
            );
            Some(match cut {
                Some((rect, radius, program)) => {
                    OutputElement::ClippedTexture(Clipped::new(element, rect, radius, program))
                }
                None => OutputElement::Texture(element),
            })
        }
    }
}

/// A picture of this size without its middle: the four strips around a
/// window of `inner` size that stands `margin` in from the picture's
/// corner, each as (x, y, width, height) in the picture; the hole is
/// `inset` smaller than the window on every side (the window's edge
/// keeps what is drawn at it). Strips of no size are left out.
pub fn around(
    whole: (i32, i32),
    margin: i32,
    inner: (i32, i32),
    inset: i32,
) -> Vec<(i32, i32, i32, i32)> {
    let left = (margin + inset).clamp(0, whole.0);
    let top = (margin + inset).clamp(0, whole.1);
    let right = (margin + inner.0 - inset).clamp(left, whole.0);
    let bottom = (margin + inner.1 - inset).clamp(top, whole.1);
    [
        (0, 0, whole.0, top),
        (0, bottom, whole.0, whole.1 - bottom),
        (0, top, left, bottom - top),
        (right, top, whole.0 - right, bottom - top),
    ]
    .into_iter()
    .filter(|(_, _, width, height)| *width > 0 && *height > 0)
    .collect()
}

impl State {
    /// What lies over the desktop while it changes to or from the lock
    /// screen, front to back: the dark at the edges, a dark veil, and the
    /// display's wallpaper blurred (without a wallpaper the veil alone,
    /// which then covers everything).
    fn veil_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
        screen: gevrek_engine::Rect,
        look: crate::lock::LockLook,
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let points: (i32, i32) = (screen.width.max(1), screen.height.max(1));
        if look.black > 0.0 {
            elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                smithay::backend::renderer::element::Id::new(),
                Rectangle::new((0, 0).into(), points.into()),
                smithay::backend::renderer::utils::CommitCounter::default(),
                Color32F::new(0.0, 0.0, 0.0, look.black),
                Kind::Unspecified,
            )));
        }
        let wanted = &self.config.desktop.animations.session;
        if look.vignette > 0.0 && wanted.vignette {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (0.0, 0.0),
                &self.vignette,
                Some(look.vignette),
                Some(Rectangle::from_size((96.0, 96.0).into())),
                Some(points.into()),
                Kind::Unspecified,
            ) {
                Ok(element) => elements.push(OutputElement::Memory(element)),
                Err(error) => log::debug!("the vignette: {error:?}"),
            }
        }
        if look.veil <= 0.0 {
            return;
        }
        // (What is behind it, soft: the desktop as it was drawn, else
        // the wallpaper; unless a plain veil is asked for.)
        let soft = wanted.blur.then(|| self.soft_behind(output)).flatten();
        // Over the blurred wallpaper as dark as the lock screen has it;
        // alone, all of the background's colour.
        let dark = look.veil * if soft.is_some() { 0.62 } else { 1.0 };
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            smithay::backend::renderer::element::Id::new(),
            Rectangle::new((0, 0).into(), points.into()),
            smithay::backend::renderer::utils::CommitCounter::default(),
            Color32F::new(
                BACKGROUND.r() * dark,
                BACKGROUND.g() * dark,
                BACKGROUND.b() * dark,
                dark,
            ),
            Kind::Unspecified,
        )));
        if let Some((soft, soft_size)) = soft {
            elements.extend(soft_element(
                renderer,
                &soft,
                (0.0, 0.0),
                Some(look.veil),
                Rectangle::from_size((f64::from(soft_size.0), f64::from(soft_size.1)).into()),
                points,
                None,
            ));
        }
    }

    /// The pointer's picture with its hot spot at `pointer`: a program's
    /// own, or the theme's for what is asked for (which the session
    /// gevrek runs in draws itself where there is one, unless `own`).
    pub(crate) fn pointer_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
        pointer: Point<i32, Physical>,
        own: bool,
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        match &self.cursor_status {
            CursorImageStatus::Surface(surface) if surface.is_alive() => {
                // The client's own picture, placed by its hot spot.
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .map(|data| {
                            data.lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .hotspot
                        })
                        .unwrap_or_default()
                });
                let at: Point<i32, Physical> =
                    (pointer.x - hotspot.x, pointer.y - hotspot.y).into();
                elements.extend(
                    render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                        renderer,
                        surface,
                        at,
                        1.0,
                        1.0,
                        Kind::Cursor,
                    )
                    .into_iter()
                    .map(OutputElement::Surface),
                );
            }
            CursorImageStatus::Hidden => {}
            // A named cursor (or a client picture that is gone): the host
            // session draws it, or gevrek draws its arrow.
            _ if own || self.draw_own_cursor => {
                // The picture of the theme for what was asked for (an
                // arrow when a client's own picture is gone), else gevrek's
                // own arrow.
                let icon = match &self.cursor_status {
                    CursorImageStatus::Named(icon) => *icon,
                    _ => smithay::input::pointer::CursorIcon::Default,
                };
                let scale = self
                    .desktop
                    .outputs
                    .iter()
                    .find(|record| record.id == output)
                    .map_or(1.0, |record| record.scale);
                let themed = self.cursors.get(icon, scale);
                let (buffer, hotspot) = match &themed {
                    Some(cursor) => (&cursor.buffer, cursor.hotspot),
                    None => (&self.arrow, (0, 0)),
                };
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (
                        f64::from(pointer.x - hotspot.0),
                        f64::from(pointer.y - hotspot.1),
                    ),
                    buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(element) => elements.push(OutputElement::Memory(element)),
                    Err(error) => log::debug!("cursor: {error:?}"),
                }
            }
            _ => {}
        }
    }

    /// Where in its picture the pointer's hot spot is, on this display.
    pub(crate) fn pointer_hotspot(&self, output: MonitorHandle) -> (i32, i32) {
        match &self.cursor_status {
            CursorImageStatus::Surface(surface) if surface.is_alive() => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<CursorImageSurfaceData>()
                        .map(|data| {
                            data.lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .hotspot
                        })
                        .unwrap_or_default()
                });
                (hotspot.x, hotspot.y)
            }
            CursorImageStatus::Hidden => (0, 0),
            status => {
                let icon = match status {
                    CursorImageStatus::Named(icon) => *icon,
                    _ => smithay::input::pointer::CursorIcon::Default,
                };
                let scale = self
                    .desktop
                    .outputs
                    .iter()
                    .find(|record| record.id == output)
                    .map_or(1.0, |record| record.scale);
                self.cursors
                    .get(icon, scale)
                    .map_or((0, 0), |cursor| cursor.hotspot)
            }
        }
    }

    /// The elements of one output, front to back, in the coordinates of
    /// that output.
    pub fn render_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
    ) -> Vec<OutputElement<R>>
    where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let Some(screen) = self.desktop.output_rectangle(output) else {
            return Vec::new();
        };
        let mut elements = Vec::new();
        // The warning that the screen is about to lock by itself: the
        // picture grows dark, the pointer too.
        if self.lock_warning > 0 && !self.locked {
            let dark = f32::from(self.lock_warning) / 20.0 * 0.7;
            elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                self.lock_warning_id.clone(),
                Rectangle::new(
                    (0, 0).into(),
                    (screen.width.max(1), screen.height.max(1)).into(),
                ),
                self.lock_warning_commit,
                Color32F::new(0.0, 0.0, 0.0, dark),
                Kind::Unspecified,
            )));
        }
        // The pointer is above everything.
        let pointer: Point<i32, Physical> = (
            self.desktop.pointer.0 - screen.x,
            self.desktop.pointer.1 - screen.y,
        )
            .into();
        // What is being dragged follows the pointer, under its picture.
        let drag_icon = self.drag_icon.as_ref().filter(|surface| surface.is_alive());
        if !self.hide_pointer {
            self.pointer_elements(renderer, output, pointer, false, &mut elements);
        }
        if let Some(icon) = drag_icon {
            elements.extend(
                render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                    renderer,
                    icon,
                    pointer,
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(OutputElement::Surface),
            );
        }
        // Locked: the lock screen's surface for this output and nothing
        // else. Nothing of the session is drawn, whatever the lock screen
        // does or fails to do.
        // (While the change to or from the lock screen is drawn, a veil
        // lies over what is shown: over the desktop until it covers it,
        // then over the lock screen, which comes out from under it.)
        let look = self.lock_look();
        // (Not in the picture of what lies behind it.)
        if let Some(look) = look.filter(|_| !self.backdrop_pass) {
            self.veil_elements(renderer, output, screen, look, &mut elements);
        }
        // (The desktop is under the veil only until it is covered, and
        // never when no change is drawn.)
        if self.locked && look.is_none_or(|look| look.covered) {
            if let Some(lock) = self.lock_surface(output) {
                elements.extend(
                    render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                        renderer,
                        lock.wl_surface(),
                        (0, 0),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )
                    .into_iter()
                    .map(OutputElement::Surface),
                );
            }
            return elements;
        }
        // X11 menus and tooltips place themselves, above everything.
        for window in self.x11_overrides.iter().rev() {
            let Some(surface) = window.wl_surface() else {
                continue;
            };
            let geometry = window.geometry();
            let at: Point<i32, Physical> =
                (geometry.loc.x - screen.x, geometry.loc.y - screen.y).into();
            elements.extend(render_elements_from_surface_tree(
                renderer,
                &surface,
                at,
                1.0,
                1.0,
                Kind::Unspecified,
            ));
        }
        // Above everything: lock screens, on-screen displays.
        self.layer_elements(renderer, output, &[Layer::Overlay], &mut elements);
        // A fullscreen window covers bars and launchers (the Top layer) and
        // every other window; it is collected apart and put between.
        let mut fullscreen: Vec<OutputElement<R>> = Vec::new();
        let mut windows: Vec<OutputElement<R>> = Vec::new();
        let mut top: Vec<OutputElement<R>> = Vec::new();
        self.layer_elements(renderer, output, &[Layer::Top], &mut top);
        for id in self.desktop.drawn_in_order().into_iter().rev() {
            let window = &self.desktop.windows[&id];
            // A window on a strip (the scrolling layout) is drawn on its
            // strip's display alone: where it reaches beyond that
            // display's edge it is cut off, and not shown on the display
            // beside it.
            if self
                .desktop
                .homes
                .get(&id)
                .is_some_and(|home| *home != output)
            {
                continue;
            }
            // On its way to its place (a workspace that slides in, a
            // window that scales in), a window is drawn only on the output
            // its place is on: it comes in from that output's edge, and is
            // not seen crossing the output beside it.
            if window.drawn.is_some() {
                let place = window.rect;
                let home = self
                    .desktop
                    .output_at(place.x + place.width / 2, place.y + place.height / 2);
                if home.is_some() && home != Some(output) {
                    continue;
                }
            }
            let elements = if window.fullscreen {
                &mut fullscreen
            } else {
                &mut windows
            };
            // What goes under the window's own content.
            let mut aura_elements: Vec<OutputElement<R>> = Vec::new();
            let Some(surface) = window.surface.wl_surface() else {
                continue;
            };
            let surface = &surface;
            let alpha = f32::from(window.drawn_opacity.unwrap_or(window.opacity)) / 255.0;
            // A window that is see-through by itself (not one that is
            // only coming or going) shows what is behind it soft, where
            // the configuration has it so; and is then no part of the
            // picture of what is behind.
            let soft_behind = window.opacity < 255
                && !window.fullscreen
                && window.overview.is_none()
                && self.config.desktop.platform.wayland.blur_behind_windows;
            if soft_behind && self.backdrop_pass {
                continue;
            }
            // Where the window is drawn: mid-transition, or where it is.
            // (In an overview: its place there, the picture made smaller.)
            let shown = window.drawn.or(window.overview).unwrap_or(window.rect);
            // A tile whose windows are tabs has a strip of tabs above the
            // window: strip and window are one frame. The border, the
            // glow and the shadow go around both, and the round corners
            // at the top are the strip's. (Drawn under the window's own
            // border, not over it: it used to lie on the border's top.)
            let bar = (self.desktop.decorations_visible
                && !window.fullscreen
                && window.overview.is_none())
            .then(|| self.desktop.stackbars.iter().find(|bar| bar.key == id))
            .flatten();
            let framed = match bar {
                Some(bar) if window.drawn.is_none() && bar.rect.y < shown.y => {
                    gevrek_engine::Rect::new(
                        shown.x,
                        bar.rect.y,
                        shown.width,
                        shown.height + (shown.y - bar.rect.y),
                    )
                }
                _ => shown,
            };
            // Where the surface starts on this output.
            let origin: Point<i32, Physical> = (
                shown.x - window.geometry_offset.0 - screen.x,
                shown.y - window.geometry_offset.1 - screen.y,
            )
                .into();

            // Popups are above their window.
            for (popup, offset) in PopupManager::popups_for_surface(surface) {
                let at: Point<i32, Physical> = match &popup {
                    PopupKind::Xdg(_) => {
                        let geometry = popup.geometry();
                        (
                            origin.x + offset.x - geometry.loc.x,
                            origin.y + offset.y - geometry.loc.y,
                        )
                            .into()
                    }
                    // An input method's window: under the text cursor.
                    PopupKind::InputMethod(method) => {
                        let (x, y) =
                            crate::input_method::under_the_cursor(method.text_input_rectangle());
                        (origin.x + x, origin.y + y).into()
                    }
                };
                elements.extend(render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    at,
                    1.0,
                    alpha,
                    Kind::Unspecified,
                ));
            }

            // The tint: a veil of colour over the window (stronger over
            // those that are not focused, by the configuration).
            let effects = &self.config.desktop.appearance.effects;
            let focused = window.border.is_some_and(|border| border.focused);
            let tint = if focused {
                effects.tint.active_opacity
            } else {
                effects.tint.inactive_opacity
            };
            if effects.tint.enabled
                && tint > 0
                && !window.fullscreen
                && self.desktop.decorations_visible
            {
                let a = f32::from(tint) / 255.0 * alpha;
                let (r, g, b) = effects.tint.color;
                elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                    window.tint_id.clone(),
                    Rectangle::new(
                        (shown.x - screen.x, shown.y - screen.y).into(),
                        (shown.width.max(1), shown.height.max(1)).into(),
                    ),
                    window.border_commit,
                    Color32F::new(
                        f32::from(r) / 255.0 * a,
                        f32::from(g) / 255.0 * a,
                        f32::from(b) / 255.0 * a,
                        a,
                    ),
                    Kind::Unspecified,
                )));
            }
            // Shadow, glow and a border that may be a gradient: one picture
            // around the window, painted by the code every backend shares
            // and kept until the window's size or look changes. It takes
            // the place of the plain border.
            let mut aura_drawn = false;
            // How round the corners of the content are cut: as round as
            // what is drawn around the window.
            let mut content_radius = 0.0f32;
            if self.desktop.decorations_visible
                && gevrek_engine::decoration_render::DecorationSpec::wanted(effects)
            {
                if let Some(border) = window
                    .border
                    .filter(|border| border.visible && !window.fullscreen)
                {
                    let (color, border_alpha) = window.drawn_border.unwrap_or(if border.focused {
                        (border.active_color, border.active_alpha)
                    } else {
                        (border.inactive_color, border.inactive_alpha)
                    });
                    // A floating window may have a glow of its own reach
                    // and strength.
                    let own_effects;
                    let effects = match window.glow {
                        Some(strength) => {
                            own_effects = gevrek_model::settings::WindowEffectsProfile {
                                glow: strength.of(&effects.glow),
                                ..effects.clone()
                            };
                            &own_effects
                        }
                        None => effects,
                    };
                    let spec = gevrek_engine::decoration_render::DecorationSpec::for_window(
                        effects,
                        gevrek_engine::decoration_render::DecoratedWindow {
                            frame: (framed.width.max(1), framed.height.max(1)),
                            border_width: border.width,
                            border_offset: border.offset,
                            border_color: color,
                            border_alpha,
                            focused: border.focused,
                        },
                        // The focused window's gradient turns.
                        if border.focused {
                            self.gradient_angle()
                        } else {
                            effects.gradient.angle
                        },
                    );
                    let mut aura = window.aura.borrow_mut();
                    if aura.as_ref().map(|(painted, _)| painted) != Some(&spec) {
                        // Only the direction changed: the border alone is
                        // coloured anew on the picture that is there.
                        let mut drawing = window.aura_drawing.borrow_mut();
                        let mut turned_in_place = false;
                        match drawing
                            .as_mut()
                            .filter(|drawing| !drawing.is_released() && drawing.shows(&spec))
                        {
                            Some(drawing) => {
                                drawing.turn(spec.gradient_angle);
                                // The picture that is there keeps all but
                                // the border's strips: only those are
                                // written, handed to the graphics card and
                                // drawn again. (A whole new picture for
                                // every step of a turning border cost a
                                // sixth of a processor on a large window.)
                                if let Some((painted, buffer)) = aura.as_mut() {
                                    if (painted.width(), painted.height())
                                        == (spec.width(), spec.height())
                                    {
                                        let strips = drawing.band_strips();
                                        let written = buffer.render().draw(|bytes| {
                                            drawing.write_band(bytes);
                                            Ok::<_, ()>(
                                                strips
                                                    .iter()
                                                    .map(|(x, y, width, height)| {
                                                        smithay::utils::Rectangle::new(
                                                            (*x, *y).into(),
                                                            (*width, *height).into(),
                                                        )
                                                    })
                                                    .collect(),
                                            )
                                        });
                                        if written.is_ok() {
                                            *painted = spec.clone();
                                            turned_in_place = true;
                                        }
                                    }
                                }
                            }
                            None => {
                                *drawing =
                                    Some(gevrek_engine::decoration_render::Prepared::new(&spec))
                            }
                        }
                        if !turned_in_place {
                            let pixels = drawing
                                .as_ref()
                                .map(|drawing| drawing.pixels())
                                .unwrap_or_default();
                            // 0xAARRGGBB with the colour already multiplied by
                            // its alpha: the bytes of an ARGB buffer.
                            let bytes: Vec<u8> = pixels
                                .iter()
                                .flat_map(|pixel| pixel.to_le_bytes())
                                .collect();
                            *aura = Some((
                                spec.clone(),
                                MemoryRenderBuffer::from_slice(
                                    &bytes,
                                    smithay::backend::allocator::Fourcc::Argb8888,
                                    (spec.width(), spec.height()),
                                    1,
                                    smithay::utils::Transform::Normal,
                                    None,
                                ),
                            ));
                        }
                    }
                    if let Some((spec, buffer)) = aura.as_ref() {
                        let corner = (
                            framed.x - screen.x - spec.margin,
                            framed.y - screen.y - spec.margin,
                        );
                        // Under a window that is see-through by itself the
                        // picture has no middle: the shadow's dark body,
                        // which a solid window hides, would stand behind
                        // the glass. Four strips around the window then.
                        let whole = (spec.width(), spec.height());
                        let parts: Vec<(i32, i32, i32, i32)> = if window.opacity < 255 {
                            around(
                                whole,
                                spec.margin,
                                (framed.width, framed.height),
                                (spec.corner_radius * 0.3).ceil() as i32 + 2,
                            )
                        } else {
                            vec![(0, 0, whole.0, whole.1)]
                        };
                        for (x, y, width, height) in parts {
                            match MemoryRenderBufferRenderElement::from_buffer(
                                renderer,
                                (f64::from(corner.0 + x), f64::from(corner.1 + y)),
                                buffer,
                                Some(alpha),
                                Some(Rectangle::new(
                                    (f64::from(x), f64::from(y)).into(),
                                    (f64::from(width), f64::from(height)).into(),
                                )),
                                Some((width, height).into()),
                                Kind::Unspecified,
                            ) {
                                Ok(element) => {
                                    aura_elements.push(OutputElement::Memory(element));
                                    aura_drawn = true;
                                    content_radius = spec.corner_radius;
                                }
                                Err(error) => log::debug!("effects: {error:?}"),
                            }
                        }
                    }
                }
            }
            // The border is drawn over the edge of the window when its
            // offset is negative, so it comes before the window.
            if self.desktop.decorations_visible && !aura_drawn {
                if let Some(border) = window
                    .border
                    .filter(|border| border.visible && border.width > 0 && !window.fullscreen)
                {
                    let (color, border_alpha) = window.drawn_border.unwrap_or(if border.focused {
                        (border.active_color, border.active_alpha)
                    } else {
                        (border.inactive_color, border.inactive_alpha)
                    });
                    let color_rgb = color;
                    let a = f32::from(border_alpha) / 255.0;
                    let color = Color32F::new(
                        f32::from(color.0) / 255.0 * a,
                        f32::from(color.1) / 255.0 * a,
                        f32::from(color.2) / 255.0 * a,
                        a,
                    );
                    // The outer rectangle of the border.
                    let grow = border.width + border.offset;
                    let (x, y) = (framed.x - screen.x - grow, framed.y - screen.y - grow);
                    let (w, h) = (framed.width + 2 * grow, framed.height + 2 * grow);
                    let t = border.width;
                    // A rounded border leaves its corners to painted tiles;
                    // the straight parts stop short of them.
                    let c = if border.rounded {
                        crate::border::tile_size(t).min(w / 2).min(h / 2)
                    } else {
                        0
                    };
                    if c > 0 {
                        // The ring is this round on its outside; the
                        // window's edge lies `grow` further in.
                        content_radius = (crate::border::tile_size(t) - grow).max(0) as f32;
                        let look = (t, color_rgb, border_alpha);
                        let mut cache = window.corner_tiles.borrow_mut();
                        if cache.as_ref().map(|tiles| tiles.look) != Some(look) {
                            let size = crate::border::tile_size(t);
                            *cache = Some(crate::desktop::CornerTiles {
                                look,
                                tiles: crate::border::CORNERS.map(|corner| {
                                    MemoryRenderBuffer::from_slice(
                                        &crate::border::corner_tile(
                                            t,
                                            color_rgb,
                                            border_alpha,
                                            corner,
                                        ),
                                        Fourcc::Argb8888,
                                        (size, size),
                                        1,
                                        smithay::utils::Transform::Normal,
                                        None,
                                    )
                                }),
                            });
                        }
                        let places = [
                            (x, y),
                            (x + w - c, y),
                            (x, y + h - c),
                            (x + w - c, y + h - c),
                        ];
                        if let Some(tiles) = cache.as_ref() {
                            for (tile, at) in tiles.tiles.iter().zip(places) {
                                match MemoryRenderBufferRenderElement::from_buffer(
                                    renderer,
                                    (f64::from(at.0), f64::from(at.1)),
                                    tile,
                                    None,
                                    None,
                                    None,
                                    Kind::Unspecified,
                                ) {
                                    Ok(element) => elements.push(OutputElement::Memory(element)),
                                    Err(error) => log::debug!("border corner: {error:?}"),
                                }
                            }
                        }
                    }
                    let pieces = if c > 0 {
                        [
                            (x + c, y, w - 2 * c, t),         // top
                            (x + c, y + h - t, w - 2 * c, t), // bottom
                            (x, y + c, t, h - 2 * c),         // left
                            (x + w - t, y + c, t, h - 2 * c), // right
                        ]
                    } else {
                        [
                            (x, y, w, t),                     // top
                            (x, y + h - t, w, t),             // bottom
                            (x, y + t, t, h - 2 * t),         // left
                            (x + w - t, y + t, t, h - 2 * t), // right
                        ]
                    };
                    use gevrek_model::graph::BorderStyle;
                    // The pieces some styles add, each with an identity of
                    // its own for damage tracking.
                    let mut extra: Vec<(crate::border::Piece, Color32F)> = Vec::new();
                    match border.style {
                        BorderStyle::Dashed => {
                            extra.extend(
                                crate::border::dashed(&pieces)
                                    .into_iter()
                                    .map(|piece| (piece, color)),
                            );
                        }
                        BorderStyle::Double => {
                            extra.extend(
                                crate::border::double((x, y, w, h), t)
                                    .into_iter()
                                    .map(|piece| (piece, color)),
                            );
                        }
                        // Square and rounded are the plain pieces; the
                        // configuration accepts no other style.
                        _ => {}
                    }
                    if !matches!(border.style, BorderStyle::Dashed | BorderStyle::Double) {
                        for (piece, id) in pieces.into_iter().zip(window.border_ids.iter()) {
                            if piece.2 <= 0 || piece.3 <= 0 {
                                continue;
                            }
                            elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                                id.clone(),
                                Rectangle::new(
                                    (piece.0, piece.1).into(),
                                    (piece.2, piece.3).into(),
                                ),
                                window.border_commit,
                                color,
                                Kind::Unspecified,
                            )));
                        }
                    }
                    let mut ids = window.extra_ids.borrow_mut();
                    while ids.len() < extra.len() {
                        ids.push(smithay::backend::renderer::element::Id::new());
                    }
                    for ((piece, piece_color), id) in extra.into_iter().zip(ids.iter()) {
                        if piece.2 <= 0 || piece.3 <= 0 {
                            continue;
                        }
                        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                            id.clone(),
                            Rectangle::new((piece.0, piece.1).into(), (piece.2, piece.3).into()),
                            window.border_commit,
                            piece_color,
                            Kind::Unspecified,
                        )));
                    }
                }
            }

            let surfaces: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
                renderer,
                surface,
                origin,
                1.0,
                alpha,
                Kind::Unspecified,
            );
            // While a transition changes the size, the content is stretched
            // to the size it is drawn at (the client draws the final size).
            let resizing = (shown.width, shown.height) != (window.rect.width, window.rect.height)
                && window.rect.width > 0
                && window.rect.height > 0;
            // Round corners: the content is cut to the window's rectangle.
            // (Not a fullscreen window's: it has the display's corners,
            // and only a picture that is not cut may go to the display
            // as it is, past gevrek's drawing.)
            let cut = (content_radius > 0.0 && !window.fullscreen)
                .then(|| renderer.clip_program())
                .flatten()
                .map(|program| {
                    let rect = Rectangle::new(
                        (framed.x - screen.x, framed.y - screen.y).into(),
                        (framed.width.max(1), framed.height.max(1)).into(),
                    );
                    (rect, program)
                });
            // The strip of tabs, cut to the same frame (its corners at
            // the top are round where the frame's are).
            let mut strip: Vec<OutputElement<R>> = Vec::new();
            if let Some(bar) = bar {
                let at = (
                    f64::from(bar.rect.x - screen.x),
                    f64::from(bar.rect.y - screen.y),
                );
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    at,
                    &bar.buffer,
                    Some(alpha),
                    None,
                    None,
                    Kind::Unspecified,
                ) {
                    Ok(element) => strip.push(match &cut {
                        Some((rect, program)) if framed != shown => OutputElement::ClippedMemory(
                            Clipped::new(element, *rect, content_radius, program.clone()),
                        ),
                        _ => OutputElement::Memory(element),
                    }),
                    Err(error) => log::debug!("stackbar: {error:?}"),
                }
            }
            if resizing {
                let scale = smithay::utils::Scale::from((
                    f64::from(shown.width) / f64::from(window.rect.width),
                    f64::from(shown.height) / f64::from(window.rect.height),
                ));
                let corner: Point<i32, Physical> = (shown.x - screen.x, shown.y - screen.y).into();
                elements.extend(surfaces.into_iter().map(|element| {
                    let element = RescaleRenderElement::from_element(element, corner, scale);
                    match &cut {
                        Some((rect, program)) => OutputElement::ClippedScaled(Clipped::new(
                            element,
                            *rect,
                            content_radius,
                            program.clone(),
                        )),
                        None => OutputElement::Scaled(element),
                    }
                }));
            } else if let Some((rect, program)) = &cut {
                elements.extend(surfaces.into_iter().map(|element| {
                    OutputElement::Clipped(Clipped::new(
                        element,
                        *rect,
                        content_radius,
                        program.clone(),
                    ))
                }));
            } else if window.overflows && !window.fullscreen {
                // A program that would not be as small as its tile: what
                // reaches beyond the tile is not drawn (nor does it take
                // the pointer, see `Desktop::window_at`).
                let tile = Rectangle::new(
                    (shown.x - screen.x, shown.y - screen.y).into(),
                    (shown.width.max(1), shown.height.max(1)).into(),
                );
                elements.extend(surfaces.into_iter().filter_map(|element| {
                    CropRenderElement::from_element(element, 1.0, tile).map(OutputElement::Cropped)
                }));
            } else {
                elements.extend(surfaces.into_iter().map(OutputElement::Surface));
            }
            elements.append(&mut strip);
            elements.append(&mut aura_elements);
            // Under all of it: the part of what lies behind, soft.
            if soft_behind && screen.width > 0 && screen.height > 0 {
                if let Some((soft, soft_size)) = self.soft_behind(output) {
                    let across = f64::from(soft_size.0) / f64::from(screen.width);
                    let down = f64::from(soft_size.1) / f64::from(screen.height);
                    let part = Rectangle::new(
                        (
                            f64::from(framed.x - screen.x) * across,
                            f64::from(framed.y - screen.y) * down,
                        )
                            .into(),
                        (
                            f64::from(framed.width.max(1)) * across,
                            f64::from(framed.height.max(1)) * down,
                        )
                            .into(),
                    );
                    // (Fading with the window while it comes or goes.)
                    let strength = window.drawn_opacity.map_or(1.0, |drawn| {
                        (f32::from(drawn) / f32::from(window.opacity.max(1))).clamp(0.0, 1.0)
                    });
                    elements.extend(soft_element(
                        renderer,
                        &soft,
                        (
                            f64::from(framed.x - screen.x),
                            f64::from(framed.y - screen.y),
                        ),
                        Some(strength),
                        part,
                        (framed.width.max(1), framed.height.max(1)),
                        cut.as_ref()
                            .map(|(rect, program)| (*rect, content_radius, program.clone())),
                    ));
                }
            }
        }
        elements.append(&mut fullscreen);
        elements.append(&mut top);
        elements.append(&mut windows);
        self.overview_elements(renderer, output, screen, &mut elements);
        self.window_overview_elements(renderer, output, screen, &mut elements);
        // Below the windows: desktop widgets, the wallpaper.
        self.layer_elements(
            renderer,
            output,
            &[Layer::Bottom, Layer::Background],
            &mut elements,
        );
        // Behind everything: gevrek's own wallpaper, and its colour.
        let wallpaper = &self.config.desktop.wallpaper;
        let record = self
            .desktop
            .outputs
            .iter()
            .find(|record| record.id == output);
        if let Some(record) = record {
            let (path, fit) = wallpaper.of(&record.name);
            let mut pictures = self.wallpapers.borrow_mut();
            // The slideshow's picture of this moment, where this display
            // shows the slideshow (the one picture where the folder has
            // none).
            let slide = pictures.slide(&wallpaper.slideshow);
            let path = match slide {
                Some(slide) if wallpaper.slides_on(&record.name) => Some(slide),
                _ => path,
            };
            // Made at the display's own pixels, and drawn over its size
            // in points: as sharp as the display is, at any scale.
            let points = (screen.width.max(1), screen.height.max(1));
            let scale = record.scale.max(0.1);
            let size = (
                ((f64::from(points.0) * scale).round() as i32).max(1),
                ((f64::from(points.1) * scale).round() as i32).max(1),
            );
            // Made (and, when it changed, the one before set aside) first.
            let has_picture = pictures
                .picture(output, path.clone(), fit, size, scale, wallpaper.color)
                .is_some();
            if !self.config.desktop.animations.enabled {
                pictures.settle();
            }
            // While a new one comes in: it is drawn as solid as it has
            // come, over the one before (or over the colour).
            let (solid, before) = match pictures.fading(output) {
                Some((part, before)) => (part, before.cloned()),
                None => (1.0, None),
            };
            // An overview is up: the windows' pictures stand on the
            // wallpaper blurred and a little darker, so that they are
            // what is looked at.
            if !self.desktop.decorations_visible {
                if let Some((soft, soft_size)) = pictures.blurred(output) {
                    elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                        pictures.veil_id(),
                        Rectangle::new((0, 0).into(), points.into()),
                        self.backdrop_commit,
                        Color32F::new(0.0, 0.0, 0.0, 0.3),
                        Kind::Unspecified,
                    )));
                    match MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (0.0, 0.0),
                        &soft,
                        None,
                        // All of the small picture, over the display.
                        Some(Rectangle::from_size(
                            (f64::from(soft_size.0), f64::from(soft_size.1)).into(),
                        )),
                        Some(points.into()),
                        Kind::Unspecified,
                    ) {
                        Ok(element) => elements.push(OutputElement::Memory(element)),
                        Err(error) => log::debug!("the overview's wallpaper: {error:?}"),
                    }
                }
            }
            let mut put =
                |buffer: &MemoryRenderBuffer, alpha: f32, elements: &mut Vec<OutputElement<R>>| {
                    match MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        (0.0, 0.0),
                        buffer,
                        Some(alpha),
                        // All of the picture, on the display's points.
                        Some(Rectangle::from_size(
                            (f64::from(size.0), f64::from(size.1)).into(),
                        )),
                        Some(points.into()),
                        Kind::Unspecified,
                    ) {
                        Ok(element) => elements.push(OutputElement::Memory(element)),
                        Err(error) => log::debug!("wallpaper: {error:?}"),
                    }
                };
            if has_picture {
                if let Some(buffer) = pictures
                    .picture(output, path, fit, size, scale, wallpaper.color)
                    .cloned()
                {
                    put(&buffer, solid, &mut elements);
                }
            }
            if let Some(before) = &before {
                // (Without a new picture, the one before fades out.)
                put(
                    before,
                    if has_picture { 1.0 } else { 1.0 - solid },
                    &mut elements,
                );
            }
        }
        let (red, green, blue) = wallpaper.color;
        elements.push(OutputElement::Solid(SolidColorRenderElement::new(
            self.backdrop_id.clone(),
            Rectangle::new(
                (0, 0).into(),
                (screen.width.max(1), screen.height.max(1)).into(),
            ),
            self.backdrop_commit,
            Color32F::new(
                f32::from(red) / 255.0,
                f32::from(green) / 255.0,
                f32::from(blue) / 255.0,
                1.0,
            ),
            Kind::Unspecified,
        )));
        elements
    }

    /// The overview of one workspace's windows, as far as it is on this
    /// display: under each picture its window's title, and a frame around
    /// the picture that is pointed at (`window_overview.rs`).
    fn window_overview_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
        screen: gevrek_engine::Rect,
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let accent = self.core.wallpaper_accent.unwrap_or((64, 224, 208));
        let accent = Color32F::new(
            f32::from(accent.0) / 255.0,
            f32::from(accent.1) / 255.0,
            f32::from(accent.2) / 255.0,
            1.0,
        );
        for shown in &self.window_overview.shown {
            let middle = (
                shown.rect.x + shown.rect.width / 2,
                shown.rect.y + shown.rect.height / 2,
            );
            if self.desktop.output_at(middle.0, middle.1) != Some(output) {
                continue;
            }
            let (x, y) = (shown.rect.x - screen.x, shown.rect.y - screen.y);
            let (w, h) = (shown.rect.width, shown.rect.height);
            let t = crate::window_overview::MARK;
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (
                    f64::from(x + (w - shown.label_size.0) / 2),
                    f64::from(y + h + t + 2),
                ),
                &shown.label,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(element) => elements.push(OutputElement::Memory(element)),
                Err(error) => log::debug!("a window's title in the overview: {error:?}"),
            }
            if self.window_overview.marked == Some(shown.window) {
                for (id, rect) in [
                    (0, (x - t, y - t, w + 2 * t, t)),
                    (1, (x - t, y + h, w + 2 * t, t)),
                    (2, (x - t, y, t, h)),
                    (3, (x + w, y, t, h)),
                ] {
                    elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                        shown.ids[id].clone(),
                        Rectangle::new(
                            (rect.0, rect.1).into(),
                            (rect.2.max(1), rect.3.max(1)).into(),
                        ),
                        self.backdrop_commit,
                        accent,
                        Kind::Unspecified,
                    )));
                }
            }
        }
    }

    /// The overview of every workspace, as far as it is on this display:
    /// under each workspace's windows the ground of its picture, its name
    /// above it, and a frame around the one that is shown.
    fn overview_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
        screen: gevrek_engine::Rect,
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let Some(overview) = &self.workspace_overview else {
            return;
        };
        let accent = self.core.wallpaper_accent.unwrap_or((64, 224, 208));
        let accent = Color32F::new(
            f32::from(accent.0) / 255.0,
            f32::from(accent.1) / 255.0,
            f32::from(accent.2) / 255.0,
            1.0,
        );
        for cell in overview.cells.iter().filter(|cell| cell.output == output) {
            let (x, y) = (cell.rect.x - screen.x, cell.rect.y - screen.y);
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                (f64::from(x), f64::from(y - crate::overview::LABEL)),
                &cell.label,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(element) => elements.push(OutputElement::Memory(element)),
                Err(error) => log::debug!("a workspace's name in the overview: {error:?}"),
            }
            let mut solid = |id: usize, rect: (i32, i32, i32, i32), colour: Color32F| {
                elements.push(OutputElement::Solid(SolidColorRenderElement::new(
                    cell.ids[id].clone(),
                    Rectangle::new(
                        (rect.0, rect.1).into(),
                        (rect.2.max(1), rect.3.max(1)).into(),
                    ),
                    self.backdrop_commit,
                    colour,
                    Kind::Unspecified,
                )));
            };
            // The frame of the workspace that is shown, then the ground.
            if cell.shown {
                let (w, h, t) = (cell.rect.width, cell.rect.height, 2);
                solid(1, (x - t, y - t, w + 2 * t, t), accent);
                solid(2, (x - t, y + h, w + 2 * t, t), accent);
                solid(3, (x - t, y, t, h), accent);
                solid(4, (x + w, y, t, h), accent);
            }
            solid(
                0,
                (x, y, cell.rect.width, cell.rect.height),
                Color32F::new(0.0, 0.0, 0.0, 0.35),
            );
        }
    }

    /// The shadow under a surface of a shell: drawn once for the size and
    /// the look it has, and again when either is another.
    fn layer_shadow<R>(
        &self,
        renderer: &mut R,
        surface: &WlSurface,
        frame: Rectangle<i32, Physical>,
        radius: f32,
        solid: f32,
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        use gevrek_engine::decoration_render::{DecorationSpec, Prepared};
        let Some(spec) = DecorationSpec::shadow_under(
            &self.config.desktop.appearance.effects,
            (frame.size.w, frame.size.h),
            radius,
        ) else {
            return;
        };
        let mut shadows = self.layer_shadows.borrow_mut();
        let known = shadows
            .iter()
            .position(|(owner, _, _)| owner.upgrade().is_ok_and(|owner| owner == *surface));
        let index = match known {
            Some(index) if shadows[index].1 == spec => index,
            other => {
                let bytes: Vec<u8> = Prepared::new(&spec)
                    .pixels()
                    .iter()
                    .flat_map(|pixel| pixel.to_le_bytes())
                    .collect();
                let buffer = MemoryRenderBuffer::from_slice(
                    &bytes,
                    smithay::backend::allocator::Fourcc::Argb8888,
                    (spec.width(), spec.height()),
                    1,
                    smithay::utils::Transform::Normal,
                    None,
                );
                let entry = (surface.downgrade(), spec, buffer);
                match other {
                    Some(index) => {
                        shadows[index] = entry;
                        index
                    }
                    None => {
                        shadows.push(entry);
                        shadows.len() - 1
                    }
                }
            }
        };
        let (_, spec, buffer) = &shadows[index];
        match MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (
                f64::from(frame.loc.x - spec.margin),
                f64::from(frame.loc.y - spec.margin),
            ),
            buffer,
            Some(solid),
            None,
            None,
            Kind::Unspecified,
        ) {
            Ok(element) => elements.push(OutputElement::Memory(element)),
            Err(error) => log::debug!("shadow under a surface: {error:?}"),
        }
    }

    /// The layer surfaces of an output on `layers`, front to back.
    fn layer_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
        layers: &[Layer],
        elements: &mut Vec<OutputElement<R>>,
    ) where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let display = output;
        let Some((_, output)) = self.outputs.iter().find(|(id, _)| *id == output) else {
            return;
        };
        // The display's size in points, for where a part of its blurred
        // wallpaper is taken from.
        let points = self
            .desktop
            .output_rectangle(display)
            .map(|screen| (screen.width.max(1), screen.height.max(1)));
        let map = layer_map_for_output(output);
        let rules = &self.config.desktop.layer_rules;
        // (The shadows of surfaces that are gone are let go.)
        self.layer_shadows
            .borrow_mut()
            .retain(|(surface, _, _)| surface.upgrade().is_ok());
        for layer in layers {
            for surface in map.layers_on(*layer).rev() {
                let Some(geometry) = map.layer_geometry(surface) else {
                    continue;
                };
                // (What shows what is behind it is no part of the picture
                // of what is behind.)
                if self.backdrop_pass
                    && gevrek_model::layer_rules::blur_of(rules, surface.namespace())
                {
                    continue;
                }
                let at: Point<i32, Physical> = (geometry.loc.x, geometry.loc.y).into();
                let radius =
                    gevrek_model::layer_rules::corner_radius_of(rules, surface.namespace()) as f32;
                // How solid the configuration's rules draw a surface of
                // this name (its menus as it draws them).
                let solid = gevrek_model::layer_rules::opacity_of(
                    &self.config.desktop.layer_rules,
                    surface.namespace(),
                );
                for (popup, offset) in PopupManager::popups_for_surface(surface.wl_surface()) {
                    let popup_at: Point<i32, Physical> = match &popup {
                        PopupKind::Xdg(_) => {
                            let popup_geometry = popup.geometry();
                            (
                                at.x + offset.x - popup_geometry.loc.x,
                                at.y + offset.y - popup_geometry.loc.y,
                            )
                                .into()
                        }
                        // An input method's window, for a text field on
                        // a panel or a menu: under the text cursor.
                        PopupKind::InputMethod(method) => {
                            let (x, y) = crate::input_method::under_the_cursor(
                                method.text_input_rectangle(),
                            );
                            (at.x + x, at.y + y).into()
                        }
                    };
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        popup.wl_surface(),
                        popup_at,
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    ));
                }
                let own: Vec<WaylandSurfaceRenderElement<R>> = render_elements_from_surface_tree(
                    renderer,
                    surface.wl_surface(),
                    at,
                    1.0,
                    solid,
                    Kind::Unspecified,
                );
                // Round corners, where its rule asks for them: the
                // surface is cut to its rectangle.
                let frame =
                    Rectangle::new(at, (geometry.size.w.max(1), geometry.size.h.max(1)).into());
                match (radius > 0.0).then(|| renderer.clip_program()).flatten() {
                    Some(program) => elements.extend(own.into_iter().map(|element| {
                        OutputElement::Clipped(Clipped::new(
                            element,
                            frame,
                            radius,
                            program.clone(),
                        ))
                    })),
                    None => elements.extend(own.into_iter().map(OutputElement::Surface)),
                }
                // Under it, where its rule asks for that: the windows'
                // shadow, as strong as the surface is solid.
                if gevrek_model::layer_rules::shadow_of(rules, surface.namespace()) {
                    self.layer_shadow(
                        renderer,
                        surface.wl_surface(),
                        frame,
                        radius,
                        solid,
                        elements,
                    );
                }
                // Behind it, where its rule asks for that: the part of
                // what lies there (the windows and the wallpaper), soft.
                if gevrek_model::layer_rules::blur_of(
                    &self.config.desktop.layer_rules,
                    surface.namespace(),
                ) {
                    let soft = self.soft_behind(display);
                    if let (Some((soft, soft_size)), Some(points)) = (soft, points) {
                        // The same part of the small picture as the
                        // surface is of the display.
                        let across = f64::from(soft_size.0) / f64::from(points.0);
                        let down = f64::from(soft_size.1) / f64::from(points.1);
                        let part = Rectangle::new(
                            (
                                f64::from(geometry.loc.x) * across,
                                f64::from(geometry.loc.y) * down,
                            )
                                .into(),
                            (
                                f64::from(geometry.size.w.max(1)) * across,
                                f64::from(geometry.size.h.max(1)) * down,
                            )
                                .into(),
                        );
                        elements.extend(soft_element(
                            renderer,
                            &soft,
                            (f64::from(at.x), f64::from(at.y)),
                            None,
                            part,
                            (geometry.size.w.max(1), geometry.size.h.max(1)),
                            None,
                        ));
                    }
                }
            }
        }
    }

    /// The direction of the focused window's border gradient now: the
    /// configured one, turned by the time when the configuration makes it
    /// turn (`spin_seconds`) and things are allowed to move by themselves
    /// (animations are on). In steps, as on Windows: at most 120 to a
    /// turn and at most twenty a second, each one being a new picture.
    pub fn gradient_angle(&self) -> f32 {
        let gradient = &self.config.desktop.appearance.effects.gradient;
        let base = gradient.angle;
        if !gradient.enabled
            || gradient.spin_seconds <= 0.0
            || !self.config.desktop.animations.enabled
        {
            return base;
        }
        let turn = self.start_time.elapsed().as_secs_f32() / gradient.spin_seconds;
        let steps = (gradient.spin_seconds * 20.0).clamp(8.0, 120.0).floor();
        base + (turn.fract() * steps).floor() * (360.0 / steps)
    }

    /// The elements of an output for the renderer: `render_elements` are
    /// in logical pixels, these are in the device pixels of the output.
    /// Clients draw at the scale of the output, so their content stays
    /// sharp, and so does what gevrek paints itself at that scale (stackbar
    /// titles, see `stackbar::paint`).
    pub fn scaled_elements<R>(
        &self,
        renderer: &mut R,
        output: MonitorHandle,
    ) -> Vec<RescaleRenderElement<OutputElement<R>>>
    where
        R: Renderer + ImportAll + ImportMem + ClipRenderer,
        R::TextureId: Send + Clone + 'static,
    {
        let scale = self
            .desktop
            .outputs
            .iter()
            .find(|record| record.id == output)
            .map_or(1.0, |record| record.scale);
        self.render_elements(renderer, output)
            .into_iter()
            .map(|element| RescaleRenderElement::from_element(element, (0, 0).into(), scale))
            .collect()
    }

    /// Tell the clients whose windows are shown that a frame was drawn, so
    /// they draw the next one.
    /// The feedback the programs with a surface on `output` asked for
    /// about the frame that is drawn now: to be told `presented` when the
    /// frame is on the screen.
    pub fn presentation_feedback(
        &self,
        output: MonitorHandle,
    ) -> Option<smithay::desktop::utils::OutputPresentationFeedback> {
        use smithay::desktop::utils::{
            take_presentation_feedback_surface_tree, OutputPresentationFeedback,
        };
        use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
        let (_, wl_output) = self.outputs.iter().find(|(id, _)| *id == output)?;
        let mut feedback = OutputPresentationFeedback::new(wl_output);
        let mut take = |surface: &WlSurface| {
            take_presentation_feedback_surface_tree(
                surface,
                &mut feedback,
                |_, _| Some(wl_output.clone()),
                |_, _| Kind::Vsync,
            )
        };
        for id in self.desktop.drawn_in_order() {
            if self.surface_outputs.get(&id).map(|(on, _)| *on) != Some(output) {
                continue;
            }
            let Some(surface) = self.desktop.windows[&id].surface.wl_surface() else {
                continue;
            };
            take(&surface);
            for (popup, _) in PopupManager::popups_for_surface(&surface) {
                take(popup.wl_surface());
            }
        }
        for layer in layer_map_for_output(wl_output).layers() {
            take(layer.wl_surface());
            for (popup, _) in PopupManager::popups_for_surface(layer.wl_surface()) {
                take(popup.wl_surface());
            }
        }
        if let Some(lock) = self.lock_surface(output) {
            take(lock.wl_surface());
        }
        Some(feedback)
    }

    /// Where no display says when a frame reached it (a window in another
    /// session, outputs in memory): the frame counts as shown now.
    pub fn presented_now(&self) {
        use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
        use smithay::utils::{Clock, Monotonic};
        use smithay::wayland::presentation::Refresh;
        let now = Clock::<Monotonic>::new().now();
        for (id, _) in &self.outputs {
            if let Some(mut feedback) = self.presentation_feedback(*id) {
                feedback.presented::<_, Monotonic>(now, Refresh::Unknown, 0, Kind::empty());
            }
        }
    }

    /// The programs whose windows are shown are told that a frame was
    /// drawn, and the commits they held back for it go on.
    pub fn send_frame_callbacks(&mut self) {
        // (`wp-fifo`, `wp-commit-timing`: every surface that is told of
        // the frame is also let go on, a panel's as much as a window's —
        // the graphics library of any program may use them.)
        let waiting = self.send_frames_to_all();
        for client in waiting {
            use smithay::wayland::compositor::CompositorHandler;
            let display = self.display.clone();
            self.client_compositor_state(&client)
                .blocker_cleared(self, &display);
        }
    }

    /// How long until the earliest picture that a program holds back for
    /// a time of its own (`wp-commit-timing`) is due: nothing when no
    /// picture waits, no time at all when one is due already. Such a
    /// picture is let go when the programs are told of a frame, and
    /// nothing tells gevrek that one came in: whoever paces the frames
    /// asks here and wakes up in time.
    pub fn paced_wait(&self) -> Option<std::time::Duration> {
        use smithay::utils::{Monotonic, Time};
        use smithay::wayland::commit_timing::CommitTimerBarrierStateUserData;
        let mut earliest: Option<smithay::wayland::commit_timing::Timestamp> = None;
        let mut look = |surface: &WlSurface| {
            with_surface_tree_downward(
                surface,
                (),
                |_, _, &()| TraversalAction::DoChildren(()),
                |_, states, &()| {
                    let Some(timers) = states.data_map.get::<CommitTimerBarrierStateUserData>()
                    else {
                        return;
                    };
                    let timers = timers
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if let Some(deadline) = timers.next_deadline() {
                        if earliest.is_none_or(|known| deadline < known) {
                            earliest = Some(deadline);
                        }
                    }
                },
                |_, _, &()| true,
            );
        };
        for (_, lock) in &self.lock_surfaces {
            look(lock.wl_surface());
        }
        for (_, output) in &self.outputs {
            for layer in layer_map_for_output(output).layers() {
                look(layer.wl_surface());
            }
        }
        for id in self.desktop.drawn_in_order() {
            if let Some(surface) = self.desktop.windows[&id].surface.wl_surface() {
                look(&surface);
            }
        }
        let deadline: Time<Monotonic> = earliest?.into();
        let now = smithay::utils::Clock::<Monotonic>::new().now();
        Some(Time::elapsed(&now, deadline))
    }

    /// Tell every surface that is shown of the frame; the programs whose
    /// held commits were let go by it.
    fn send_frames_to_all(&self) -> Vec<smithay::reexports::wayland_server::Client> {
        let time = self.start_time.elapsed().as_millis() as u32;
        let now = smithay::utils::Clock::<smithay::utils::Monotonic>::new().now();
        let mut waiting: Vec<smithay::reexports::wayland_server::Client> = Vec::new();
        let mut send_frames = |surface: &WlSurface, time: u32| {
            send_frames(surface, time);
            if release_paced(surface, now) {
                if let Some(client) = surface.client() {
                    if !waiting.contains(&client) {
                        waiting.push(client);
                    }
                }
            }
        };
        if let CursorImageStatus::Surface(surface) = &self.cursor_status {
            send_frames(surface, time);
        }
        if let Some(icon) = &self.drag_icon {
            send_frames(icon, time);
        }
        for (_, lock) in &self.lock_surfaces {
            send_frames(lock.wl_surface(), time);
        }
        for window in &self.x11_overrides {
            if let Some(surface) = window.wl_surface() {
                send_frames(&surface, time);
            }
        }
        for (_, output) in &self.outputs {
            for layer in layer_map_for_output(output).layers() {
                send_frames(layer.wl_surface(), time);
                // A panel's menu draws on as the panel does.
                for (popup, _) in PopupManager::popups_for_surface(layer.wl_surface()) {
                    if let PopupKind::Xdg(xdg) = &popup {
                        send_frames(xdg.wl_surface(), time);
                    }
                }
            }
        }
        for id in self.desktop.drawn_in_order() {
            let Some(surface) = self.desktop.windows[&id].surface.wl_surface() else {
                continue;
            };
            let surface = &surface;
            send_frames(surface, time);
            for (popup, _) in PopupManager::popups_for_surface(surface) {
                send_frames(popup.wl_surface(), time);
            }
        }
        waiting
    }
}

fn send_frames(surface: &WlSurface, time: u32) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_, states, &()| {
            for callback in states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .frame_callbacks
                .drain(..)
            {
                callback.done(time);
            }
        },
        |_, _, &()| true,
    );
}

/// A frame was shown: the commits a program held back until then go on.
/// With `wp-fifo` a program asks that its next picture wait until the
/// one before was on the display for a frame (a game that must not run
/// ahead of the display); with `wp-commit-timing` that a picture is not
/// taken up before a time. Returns whether anything was let go (its
/// program's commits are to be looked at again).
fn release_paced(
    surface: &WlSurface,
    now: smithay::utils::Time<smithay::utils::Monotonic>,
) -> bool {
    use smithay::wayland::commit_timing::CommitTimerBarrierStateUserData;
    use smithay::wayland::fifo::FifoBarrierCachedState;
    let mut released = false;
    with_surface_tree_downward(
        surface,
        (),
        |_, _, &()| TraversalAction::DoChildren(()),
        |_, states, &()| {
            let barrier = states
                .cached_state
                .get::<FifoBarrierCachedState>()
                .current()
                .barrier
                .take();
            if let Some(barrier) = barrier {
                barrier.signal();
                released = true;
            }
            if let Some(timers) = states.data_map.get::<CommitTimerBarrierStateUserData>() {
                let mut timers = timers
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if timers.signal_until(now) {
                    released = true;
                }
            }
        },
        |_, _, &()| true,
    );
    released
}

#[cfg(test)]
mod tests {
    use super::around;

    #[test]
    fn a_picture_without_its_middle_is_four_strips_that_cover_the_rest() {
        // A picture of 140x100 around a window of 100x60, 20 in.
        let strips = around((140, 100), 20, (100, 60), 0);
        assert_eq!(
            strips,
            [
                (0, 0, 140, 20),
                (0, 80, 140, 20),
                (0, 20, 20, 60),
                (120, 20, 20, 60)
            ]
        );
        let area: i32 = strips
            .iter()
            .map(|(_, _, width, height)| width * height)
            .sum();
        assert_eq!(area, 140 * 100 - 100 * 60, "all but the window");
        // The hole smaller than the window: the window's edge is kept.
        let inset = around((140, 100), 20, (100, 60), 3);
        let area: i32 = inset
            .iter()
            .map(|(_, _, width, height)| width * height)
            .sum();
        assert_eq!(area, 140 * 100 - 94 * 54);
        // No margin: nothing above or beside, nothing of no size.
        assert!(around((100, 60), 0, (100, 60), 0).is_empty());
        // A window too small for the inset: no hole, the picture whole.
        let whole: i32 = around((50, 50), 20, (10, 10), 8)
            .iter()
            .map(|(_, _, width, height)| width * height)
            .sum();
        assert_eq!(whole, 50 * 50);
    }
}
