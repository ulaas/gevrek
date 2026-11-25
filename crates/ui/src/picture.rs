//! A frame drawn into a picture, without a window: for the tests that
//! compare how a page looks with a picture kept of it, and for looking at
//! a surface while it is worked on, on a machine whose screen is somebody
//! else's. Behind the feature `picture` (it brings a graphics device).

/// What draws a frame into a picture, without a window: a graphics
/// device that computes on the processor (so that the same picture
/// comes out on every machine that has one), and somewhere to draw.
pub struct Painter {
    device: egui_wgpu::wgpu::Device,
    queue: egui_wgpu::wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    target: egui_wgpu::wgpu::Texture,
    /// The picture's size, in pixels.
    size: (u32, u32),
}

impl Painter {
    /// One that draws pictures of this size; `None` where the machine has
    /// no graphics device that computes on the processor.
    pub fn sized(size: (u32, u32)) -> Option<Self> {
        let picture = size;
        use egui_wgpu::wgpu;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("a page's picture"),
            size: wgpu::Extent3d {
                width: picture.0,
                height: picture.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        Some(Self {
            device,
            queue,
            renderer,
            target,
            size,
        })
    }

    /// One frame of anything that is drawn into a window, with what
    /// the pointer and the keys did since the frame before.
    pub fn frame_of(
        &mut self,
        context: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        mut draw: impl FnMut(&mut egui::Ui),
    ) -> Vec<u8> {
        let picture = self.size;
        use egui_wgpu::wgpu;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(picture.0 as f32, picture.1 as f32),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = context.run_ui(input, |ui| draw(ui));
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                self.renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let freed = std::mem::take(&mut output.textures_delta.free);
        output.textures_delta.clear();
        let jobs = context.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [picture.0, picture.1],
            pixels_per_point: output.pixels_per_point,
        };
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let before =
            self.renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("a page's picture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &jobs, &screen);
        }
        // The picture out of the device's memory, row by row.
        let row = picture.0 * 4;
        let padded = row.div_ceil(256) * 256;
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("a page's picture, read back"),
            size: u64::from(padded * picture.1),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(picture.1),
                },
            },
            wgpu::Extent3d {
                width: picture.0,
                height: picture.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue
            .submit(before.into_iter().chain(std::iter::once(encoder.finish())));
        for id in &freed {
            self.renderer.free_texture(id);
        }
        read.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = read.get_mapped_range(..).expect("the picture is read back");
        let mut pixels = Vec::with_capacity((row * picture.1) as usize);
        for line in mapped.chunks(padded as usize) {
            pixels.extend_from_slice(&line[..row as usize]);
        }
        pixels
    }
}

impl Painter {
    /// The size of the pictures, in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Write a frame's pixels as a PNG file.
    pub fn save(&self, pixels: &[u8], file: &std::path::Path) -> image::ImageResult<()> {
        image::save_buffer(
            file,
            pixels,
            self.size.0,
            self.size.1,
            image::ColorType::Rgba8,
        )
    }
}
