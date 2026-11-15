//! `linux-dmabuf`: clients that draw with the graphics card hand over their
//! pictures as buffers of the card itself, without a copy through shared
//! memory. Programs that use OpenGL or Vulkan need it to use the card at
//! all; without it they fall back to drawing in software, or do not start.

use crate::state::State;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::egl::EGLDevice;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::backend::renderer::ImportDma;
use smithay::wayland::dmabuf::{
    DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
};

/// A graphics card pictures of the screen may be drawn into buffers of:
/// its device as the system numbers it, and the kinds of buffer with the
/// layouts each takes.
pub type CaptureCard = (u64, Vec<(u32, Vec<u64>)>);

/// Whether the renderer can show this buffer.
pub type ImportTest = Box<dyn FnMut(&Dmabuf) -> bool>;

/// Announce `linux-dmabuf` with the formats `renderer` can show. `import`
/// is asked about each buffer a client makes. Nothing is announced when
/// the renderer takes no such buffers (a software renderer).
///
/// `card`: the display device the backend draws with, where it knows
/// it (the hardware backend): what programs are told to draw with when
/// the renderer cannot name its device itself.
/// The graphics card the renderer draws with, as programs may open it:
/// what they are told to draw with, and what their buffers are marked
/// as made on.
pub(crate) fn device_of(
    renderer: &GlesRenderer,
    card: Option<&std::path::Path>,
) -> Option<smithay::backend::drm::DrmNode> {
    EGLDevice::device_for_display(renderer.egl_context().display())
        .ok()
        .and_then(|device| device.try_get_render_node().ok().flatten())
        .or_else(|| {
            use smithay::backend::drm::{DrmNode, NodeType};
            let node = DrmNode::from_path(card?).ok()?;
            // (The side of it programs may open, where it has one.)
            Some(
                node.node_with_type(NodeType::Render)
                    .and_then(Result::ok)
                    .unwrap_or(node),
            )
        })
}

pub(crate) fn offer(
    state: &mut State,
    renderer: &mut GlesRenderer,
    import: ImportTest,
    card: Option<&std::path::Path>,
) {
    let formats: Vec<_> = renderer.dmabuf_formats().into_iter().collect();
    if formats.is_empty() {
        log::info!("The renderer takes no dmabufs: clients draw through shared memory");
        return;
    }
    // The device clients are to draw with: the one the renderer is on.
    let device = device_of(renderer, card);
    state.dmabuf_node = device;
    let count = formats.len();
    let feedback = device.and_then(|node| {
        DmabufFeedbackBuilder::new(node.dev_id(), formats.iter().copied())
            .build()
            .ok()
    });
    let global = match feedback {
        Some(feedback) => state
            .dmabuf_state
            .create_global_with_default_feedback::<State>(&state.display, &feedback),
        // Without a device to name, the older form of the protocol.
        None => state
            .dmabuf_state
            .create_global::<State>(&state.display, formats),
    };
    state.dmabuf_global = Some(global);
    state.dmabuf_import = Some(import);
    // Pictures of the screen straight into a program's buffer of the
    // card: where the renderer says which buffers it draws into.
    state.capture_card = device.and_then(|node| {
        use smithay::backend::allocator::Fourcc;
        use smithay::backend::renderer::Bind;
        let drawn_into = Bind::<Dmabuf>::supported_formats(renderer)?;
        let kinds: Vec<(u32, Vec<u64>)> = [Fourcc::Xrgb8888, Fourcc::Argb8888]
            .into_iter()
            .map(|code| {
                let layouts: Vec<u64> = drawn_into
                    .iter()
                    .filter(|format| format.code == code)
                    .map(|format| u64::from(format.modifier))
                    .collect();
                (code as u32, layouts)
            })
            .filter(|(_, layouts)| !layouts.is_empty())
            .collect();
        (!kinds.is_empty()).then(|| (node.dev_id(), kinds))
    });
    log::info!(
        "linux-dmabuf is offered with {count} formats{}",
        device.map_or(String::new(), |node| format!(" on {node}"))
    );
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let accepted = self
            .dmabuf_import
            .as_mut()
            .is_some_and(|import| import(&dmabuf));
        if accepted {
            // Made on the card that is drawn with (programs are told no
            // other): a window's own picture of such a buffer may go to
            // the display as it is (the hardware backend, `udev.rs`).
            if let Some(node) = self.dmabuf_node {
                dmabuf.set_node(node);
            }
            if !std::mem::replace(&mut self.dmabuf_used, true) {
                log::info!("A client draws with the graphics card (its first dmabuf was taken)");
            }
            if let Err(error) = notifier.successful::<State>() {
                log::debug!("dmabuf: {error:?}");
            }
        } else {
            log::debug!("A dmabuf the renderer cannot show was refused");
            notifier.failed();
        }
    }
}

smithay::delegate_dmabuf!(State);
