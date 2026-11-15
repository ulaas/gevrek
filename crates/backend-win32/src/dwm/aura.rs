//! What gevrek draws around and over a window on Windows: the shadow, the
//! glow and the gradient border (one layered window behind the program's
//! window, with a picture from `gevrek_engine::decoration_render`), and the
//! tint (one plain layered window kept right above the program's window).
//! Neither is owned by the program's window: an owned window would tie this
//! thread's input to the program's, and a program that hangs would hold the
//! window manager.
//!
//! `WindowManager` keeps one of these per bordered window and calls `sync`
//! wherever it moves, recolours, shows or hides the border.

use gevrek_engine::decoration_render::{self as render, DecorationSpec};
use gevrek_model::settings::WindowEffectsProfile;
use std::collections::HashMap;
use std::time::Instant;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// The window classes. Both names begin with `Gevrek_Border`: everything
/// that tells gevrek's own windows from programs' goes by that beginning.
const AURA_CLASS: windows::core::PCWSTR = windows::core::w!("Gevrek_BorderAura");
const TINT_CLASS: windows::core::PCWSTR = windows::core::w!("Gevrek_BorderTint");

struct Aura {
    hwnd: HWND,
    /// The picture the window shows; drawn again only when what it should
    /// show changes, and only its border when just the gradient turned.
    look: Option<render::Prepared>,
    position: (i32, i32),
}

struct Tint {
    hwnd: HWND,
    color: (u8, u8, u8),
    alpha: u8,
    rect: (i32, i32, i32, i32),
}

/// One window's state as the border code knows it.
#[derive(Debug, Clone, Copy)]
pub struct Target {
    pub hwnd: HWND,
    /// The window the picture goes right behind (the classic border window).
    pub behind: HWND,
    /// The visible frame of the window on the screen.
    pub frame: (i32, i32, i32, i32),
    pub border_width: i32,
    pub border_offset: i32,
    pub color: (u8, u8, u8),
    pub alpha: u8,
    pub focused: bool,
    pub visible: bool,
}

pub struct Auras {
    effects: WindowEffectsProfile,
    /// The windows whose glow is not the configuration's (floating ones).
    glows: HashMap<isize, gevrek_engine::contract::GlowStrength>,
    auras: HashMap<isize, Aura>,
    tints: HashMap<isize, Tint>,
    classes_registered: bool,
    spin_started: Instant,
}

/// A copy of the window manager's state does not own the windows made
/// here: it gets the settings and nothing else.
impl Clone for Auras {
    fn clone(&self) -> Self {
        Self {
            effects: self.effects.clone(),
            ..Self::default()
        }
    }
}

impl Default for Auras {
    fn default() -> Self {
        Self {
            effects: WindowEffectsProfile::default(),
            glows: HashMap::new(),
            auras: HashMap::new(),
            tints: HashMap::new(),
            classes_registered: false,
            spin_started: Instant::now(),
        }
    }
}

unsafe extern "system" fn plain_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // The tint: one colour, kept in the window's user data.
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if !hdc.is_invalid() {
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let brush = CreateSolidBrush(COLORREF(GetWindowLongW(hwnd, GWLP_USERDATA) as u32));
                FillRect(hdc, &rect, brush);
                let _ = DeleteObject(brush);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // Never take the mouse or the focus.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl Auras {
    /// Take new settings. Returns whether they differ from the old ones (the
    /// caller then syncs every window again).
    pub fn set_effects(&mut self, effects: WindowEffectsProfile) -> bool {
        if self.effects == effects {
            return false;
        }
        self.effects = effects;
        self.spin_started = Instant::now();
        // Every picture is out of date.
        for aura in self.auras.values_mut() {
            aura.look = None;
        }
        true
    }

    pub fn effects(&self) -> &WindowEffectsProfile {
        &self.effects
    }

    /// Whether the border itself is drawn here (then the classic border
    /// window is kept invisible).
    pub fn draws_border(&self) -> bool {
        self.effects.draws_around()
    }

    /// Whether the focused window's border turns, and so must be drawn again
    /// from time to time.
    pub fn spins(&self) -> bool {
        self.effects.gradient.enabled && self.effects.gradient.spin_seconds > 0.0
    }

    fn register_classes(&mut self) {
        if self.classes_registered {
            return;
        }
        unsafe {
            for class in [AURA_CLASS, TINT_CLASS] {
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(plain_window_proc),
                    lpszClassName: class,
                    ..Default::default()
                };
                let _ = RegisterClassW(&wc);
            }
        }
        self.classes_registered = true;
    }

    /// Give one window a glow of its own reach and strength (a floating
    /// window's), or the configuration's again. Returns whether that is
    /// another than it had (its picture is then out of date).
    pub fn set_glow(
        &mut self,
        target: HWND,
        glow: Option<gevrek_engine::contract::GlowStrength>,
    ) -> bool {
        let key = target.0 as isize;
        let before = match glow {
            Some(glow) => self.glows.insert(key, glow),
            None => self.glows.remove(&key),
        };
        if before == glow {
            return false;
        }
        if let Some(aura) = self.auras.get_mut(&key) {
            aura.look = None;
        }
        true
    }

    fn spec_for(&self, target: &Target) -> DecorationSpec {
        // A floating window may have a glow of its own reach and strength.
        let own_effects;
        let effects = match self.glows.get(&(target.hwnd.0 as isize)) {
            Some(strength) => {
                own_effects = WindowEffectsProfile {
                    glow: strength.of(&self.effects.glow),
                    ..self.effects.clone()
                };
                &own_effects
            }
            None => &self.effects,
        };
        // The band is as thick as the border: what does not reach outside
        // the frame lies inside it.
        let outside = (target.border_width + target.border_offset)
            .clamp(0, target.border_width.max(0)) as f32;
        let inside = (target.border_width as f32 - outside).max(0.0);
        let color = [target.color.0, target.color.1, target.color.2];
        let gradient = effects.gradient.enabled.then(|| {
            let end = if target.focused {
                effects.gradient.active_end
            } else {
                effects.gradient.inactive_end
            };
            [end.0, end.1, end.2]
        });
        let mut angle = effects.gradient.angle;
        if self.spins() && target.focused {
            let turn = self.spin_started.elapsed().as_secs_f32() / effects.gradient.spin_seconds;
            // In steps: at most 120 to a turn (finer cannot be told apart)
            // and at most twenty a second (each one is a new picture).
            let steps = (effects.gradient.spin_seconds * 20.0)
                .clamp(8.0, 120.0)
                .floor();
            angle += (turn.fract() * steps).floor() * (360.0 / steps);
        }
        let glow_alpha = if target.focused {
            effects.glow.opacity
        } else {
            effects.glow.inactive_opacity
        };
        let shadow_alpha = if target.focused {
            effects.shadow.opacity
        } else {
            effects.shadow.inactive_opacity
        };
        DecorationSpec {
            frame_width: target.frame.2,
            frame_height: target.frame.3,
            margin: effects.margin() + outside.ceil() as i32 + 1,
            corner_radius: effects.corner_radius.0 as f32,
            border_outside: outside,
            border_inside: inside,
            border_color: color,
            border_color_end: gradient,
            gradient_angle: angle,
            border_alpha: target.alpha,
            glow: (effects.glow.enabled && glow_alpha > 0).then(|| render::Glow {
                radius: effects.glow.radius as f32,
                color: effects
                    .glow
                    .color
                    .map(|(r, g, b)| [r, g, b])
                    .unwrap_or(color),
                alpha: glow_alpha,
            }),
            shadow: (effects.shadow.enabled && shadow_alpha > 0).then_some(render::Shadow {
                radius: effects.shadow.radius as f32,
                offset_x: effects.shadow.offset_x as f32,
                offset_y: effects.shadow.offset_y as f32,
                color: [
                    effects.shadow.color.0,
                    effects.shadow.color.1,
                    effects.shadow.color.2,
                ],
                alpha: shadow_alpha,
            }),
        }
    }

    /// Bring what is drawn for one window in line with its state.
    pub fn sync(&mut self, target: Target) {
        self.sync_aura(&target);
        self.sync_tint(&target);
    }

    fn sync_aura(&mut self, target: &Target) {
        let key = target.hwnd.0 as isize;
        if !self.draws_border() {
            self.remove_aura(key);
            return;
        }
        if !target.visible || target.frame.2 <= 0 || target.frame.3 <= 0 {
            if let Some(aura) = self.auras.get(&key) {
                unsafe {
                    let _ = ShowWindow(aura.hwnd, SW_HIDE);
                }
            }
            return;
        }
        self.register_classes();
        let spec = self.spec_for(target);
        let turns = self.spins();
        let position = (target.frame.0 - spec.margin, target.frame.1 - spec.margin);
        let aura = self.auras.entry(key).or_insert_with(|| unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                AURA_CLASS,
                windows::core::w!(""),
                WS_POPUP,
                position.0,
                position.1,
                spec.width(),
                spec.height(),
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )
            .unwrap_or_default();
            Aura {
                hwnd,
                look: None,
                position: (i32::MIN, i32::MIN),
            }
        });
        if aura.hwnd.is_invalid() {
            return;
        }
        unsafe {
            let redraw = match aura.look.as_mut() {
                Some(look) if look.shows(&spec) => {
                    if look.spec().gradient_angle == spec.gradient_angle {
                        false
                    } else if look.is_released() {
                        aura.look = Some(render::Prepared::new(&spec));
                        true
                    } else {
                        look.turn(spec.gradient_angle);
                        true
                    }
                }
                _ => {
                    aura.look = Some(render::Prepared::new(&spec));
                    true
                }
            };
            if redraw {
                if let Some(look) = aura.look.as_mut() {
                    present(aura.hwnd, position, look);
                    // The window has the picture now. The copy is kept only
                    // where the border turns (the focused window).
                    if !(turns && target.focused) {
                        look.release();
                    }
                }
                aura.position = position;
            } else if aura.position != position {
                let _ = SetWindowPos(
                    aura.hwnd,
                    None,
                    position.0,
                    position.1,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                aura.position = position;
            }
            // Right behind the border window (which is right behind the
            // program's window), and shown.
            let _ = SetWindowPos(
                aura.hwnd,
                target.behind,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    fn sync_tint(&mut self, target: &Target) {
        let key = target.hwnd.0 as isize;
        let tint = self.effects.tint.clone();
        let alpha = if !tint.enabled {
            0
        } else if target.focused {
            tint.active_opacity
        } else {
            tint.inactive_opacity
        };
        if alpha == 0 {
            self.remove_tint(key);
            return;
        }
        if !target.visible || target.frame.2 <= 0 || target.frame.3 <= 0 {
            if let Some(existing) = self.tints.get(&key) {
                unsafe {
                    let _ = ShowWindow(existing.hwnd, SW_HIDE);
                }
            }
            return;
        }
        self.register_classes();
        let color = tint.color;
        let radius = self.effects.corner_radius.0 as i32;
        // A tint whose window went away with its owner is made anew.
        if let Some(existing) = self.tints.get(&key) {
            if !unsafe { IsWindow(existing.hwnd).as_bool() } {
                self.tints.remove(&key);
            }
        }
        let entry = self.tints.entry(key).or_insert_with(|| unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                TINT_CLASS,
                windows::core::w!(""),
                WS_POPUP,
                target.frame.0,
                target.frame.1,
                target.frame.2,
                target.frame.3,
                None,
                None,
                HINSTANCE(std::ptr::null_mut()),
                None,
            )
            .unwrap_or_default();
            Tint {
                hwnd,
                color: (0, 0, 0),
                alpha: 0,
                rect: (0, 0, 0, 0),
            }
        });
        if entry.hwnd.is_invalid() {
            return;
        }
        unsafe {
            if entry.color != color || entry.alpha != alpha {
                let colorref = (color.2 as u32) << 16 | (color.1 as u32) << 8 | color.0 as u32;
                SetWindowLongW(entry.hwnd, GWLP_USERDATA, colorref as i32);
                let _ = SetLayeredWindowAttributes(entry.hwnd, COLORREF(0), alpha, LWA_ALPHA);
                let _ = InvalidateRect(entry.hwnd, None, true);
                entry.color = color;
                entry.alpha = alpha;
            }
            if entry.rect != target.frame {
                let _ = SetWindowPos(
                    entry.hwnd,
                    None,
                    target.frame.0,
                    target.frame.1,
                    target.frame.2,
                    target.frame.3,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let region = if radius > 0 {
                    CreateRoundRectRgn(
                        0,
                        0,
                        target.frame.2 + 1,
                        target.frame.3 + 1,
                        radius * 2,
                        radius * 2,
                    )
                } else {
                    CreateRectRgn(0, 0, target.frame.2, target.frame.3)
                };
                let _ = SetWindowRgn(entry.hwnd, region, true);
                entry.rect = target.frame;
            }
            // Right above its window and nowhere higher: a tint that came
            // to the top would lie over other windows that overlap its own
            // (a floating window over a tiled one).
            let above_owner = GetWindow(target.hwnd, GW_HWNDPREV).unwrap_or_default();
            if above_owner != entry.hwnd {
                let _ = SetWindowPos(
                    entry.hwnd,
                    if above_owner.is_invalid() {
                        HWND_TOP
                    } else {
                        above_owner
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            } else if !IsWindowVisible(entry.hwnd).as_bool() {
                let _ = ShowWindow(entry.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    pub fn set_visible(&self, target: HWND, visible: bool) {
        let key = target.0 as isize;
        let command = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        unsafe {
            if let Some(aura) = self.auras.get(&key) {
                let _ = ShowWindow(aura.hwnd, command);
            }
            if let Some(tint) = self.tints.get(&key) {
                let _ = ShowWindow(tint.hwnd, command);
            }
        }
    }

    pub fn set_all_visible(&self, visible: bool) {
        let command = if visible { SW_SHOWNOACTIVATE } else { SW_HIDE };
        unsafe {
            for aura in self.auras.values() {
                let _ = ShowWindow(aura.hwnd, command);
            }
            for tint in self.tints.values() {
                let _ = ShowWindow(tint.hwnd, command);
            }
        }
    }

    fn remove_aura(&mut self, key: isize) {
        if let Some(aura) = self.auras.remove(&key) {
            unsafe {
                if IsWindow(aura.hwnd).as_bool() {
                    let _ = DestroyWindow(aura.hwnd);
                }
            }
        }
    }

    fn remove_tint(&mut self, key: isize) {
        if let Some(tint) = self.tints.remove(&key) {
            unsafe {
                if IsWindow(tint.hwnd).as_bool() {
                    let _ = DestroyWindow(tint.hwnd);
                }
            }
        }
    }

    /// The window is no longer decorated.
    pub fn remove(&mut self, target: HWND) {
        let key = target.0 as isize;
        self.glows.remove(&key);
        self.remove_aura(key);
        self.remove_tint(key);
    }

    /// Remove everything (the WM stops, or all borders are destroyed).
    pub fn clear(&mut self) {
        let keys: Vec<isize> = self
            .auras
            .keys()
            .chain(self.tints.keys())
            .copied()
            .collect();
        for key in keys {
            self.remove_aura(key);
            self.remove_tint(key);
        }
    }

    /// Remove what was drawn for windows that `keep` does not name any more.
    pub fn retain(&mut self, keep: impl Fn(isize) -> bool) {
        let gone: Vec<isize> = self
            .auras
            .keys()
            .chain(self.tints.keys())
            .copied()
            .filter(|key| !keep(*key))
            .collect();
        for key in gone {
            self.remove_aura(key);
            self.remove_tint(key);
        }
    }

    /// Whether `hwnd` is one of the windows made here.
    pub fn owns(&self, hwnd: HWND) -> bool {
        self.auras.values().any(|aura| aura.hwnd == hwnd)
            || self.tints.values().any(|tint| tint.hwnd == hwnd)
    }

    pub fn count(&self) -> (usize, usize) {
        (self.auras.len(), self.tints.len())
    }
}

/// Put a picture into the layered window at `position`.
unsafe fn present(hwnd: HWND, position: (i32, i32), look: &render::Prepared) {
    let width = look.spec().width();
    let height = look.spec().height();
    if width <= 0 || height <= 0 {
        return;
    }
    let pixels = look.pixels();
    let screen = GetDC(None);
    let memory = CreateCompatibleDC(screen);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height, // rows from the top
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    if let Ok(bitmap) = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, None, 0) {
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
            let previous = SelectObject(memory, bitmap);
            let destination = POINT {
                x: position.0,
                y: position.1,
            };
            let size = SIZE {
                cx: width,
                cy: height,
            };
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = UpdateLayeredWindow(
                hwnd,
                screen,
                Some(&destination),
                Some(&size),
                memory,
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            SelectObject(memory, previous);
        }
        let _ = DeleteObject(bitmap);
    }
    let _ = DeleteDC(memory);
    ReleaseDC(None, screen);
}
