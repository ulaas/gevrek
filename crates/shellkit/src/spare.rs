//! What draws a surface, kept when the surface is closed for the next
//! one that opens.
//!
//! A menu, a hint, a list under a button: each is a surface that lives
//! for a moment, and each had what draws it made anew (its shaders put
//! together, its pipeline built) when it opened. A few of those are kept
//! now, emptied of the pictures they held, and the next surface of the
//! same kind of picture takes one.

use egui_wgpu::wgpu;

/// How many are kept at the most: a menu, its submenu and a hint are open
/// together, seldom more.
const MOST: usize = 3;

#[derive(Default)]
pub(crate) struct Spares {
    kept: Vec<(wgpu::TextureFormat, egui_wgpu::Renderer)>,
}

impl Spares {
    /// What draws a surface whose pictures are of this format: one that
    /// was kept, or a new one.
    pub fn take(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> egui_wgpu::Renderer {
        match self.kept.iter().position(|(kept, _)| *kept == format) {
            Some(index) => {
                log::trace!("a surface draws with what a closed one drew with");
                self.kept.swap_remove(index).1
            }
            None => egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default()),
        }
    }

    /// Keep what drew a surface that is gone, where there is room for
    /// it: emptied of the pictures it held (`textures`: every one that
    /// was given to it and not taken away since), so that the next
    /// surface begins with none.
    pub fn keep(
        &mut self,
        format: wgpu::TextureFormat,
        mut renderer: egui_wgpu::Renderer,
        textures: impl IntoIterator<Item = egui::TextureId>,
    ) {
        if self.kept.len() >= MOST {
            return;
        }
        for texture in textures {
            renderer.free_texture(&texture);
        }
        self.kept.push((format, renderer));
    }

    /// Let go of all of them (the device they draw with is going).
    pub fn clear(&mut self) {
        self.kept.clear();
    }
}
