//! GPUI element that displays frames produced by the player (zero-copy D3D11 surface, software fallback; see docs/player/video-surface.md).
//!
//! GPU path: each shared texture of the player's ring is registered with the patched
//! `gpui-pre-windows` and wrapped in a marker [`RenderImage`]; painting that image samples the
//! texture directly. Software path: each frame is a regular `RenderImage`.

use std::rc::Rc;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{App, Bounds, Context, Corners, Pixels, RenderImage, Task, Window, canvas, div};
use player::Frame;

/// Hides the mouse cursor until it moves, e.g. when the player's controls hide (patched GPUI).
pub use gpui_windows::hide_cursor_until_mouse_moves;

/// Magic understood by the patched GPUI atlas (vendor/gpui-pre-windows), followed by the handle.
const EXTERNAL_TEXTURE_MAGIC: &[u8; 8] = b"GPUIEXT1";

/// Called with the video area's size in physical pixels whenever it changes.
pub type ResizeHandler = Rc<dyn Fn(u32, u32)>;

/// Displays a player's frames, letterboxed by mpv to the element's size.
pub struct VideoView {
  on_resize: ResizeHandler,
  generation: u64,
  ring: Vec<(u64, Arc<RenderImage>)>,
  current_slot: Option<usize>,
  cpu_frame: Option<Arc<RenderImage>>,
  /// Images to release from the atlas at the next render (needs the window).
  stale: Vec<Arc<RenderImage>>,
  /// Radii of the picture's corners: a parent's `overflow_hidden` clips to a rectangle.
  corner_radii: Corners<Pixels>,
  _frames: Task<()>,
}

impl VideoView {
  /// Starts consuming `frames`; `on_resize` should forward the size to the player.
  pub fn new(
    frames: async_channel::Receiver<Frame>,
    on_resize: ResizeHandler,
    cx: &mut Context<Self>,
  ) -> Self {
    let task = cx.spawn(async move |this, cx| {
      while let Ok(first) = frames.recv().await {
        let mut batch = vec![first];
        while let Ok(more) = frames.try_recv() {
          batch.push(more);
        }
        let alive = this.update(cx, |view, cx| {
          for frame in batch {
            view.apply(frame);
          }
          cx.notify();
        });
        if alive.is_err() {
          break;
        }
      }
    });
    cx.on_release(|view, cx| view.release_all(cx)).detach();
    Self {
      on_resize,
      generation: 0,
      ring: Vec::new(),
      current_slot: None,
      cpu_frame: None,
      stale: Vec::new(),
      corner_radii: Corners::default(),
      _frames: task,
    }
  }

  /// Rounds the picture's corners (and the black behind it).
  pub fn set_corner_radii(&mut self, radii: Corners<Pixels>) {
    self.corner_radii = radii;
  }

  /// Replaces the resize handler (e.g. once the player exists).
  pub fn set_resize_handler(&mut self, on_resize: ResizeHandler) {
    self.on_resize = on_resize;
  }

  fn apply(&mut self, frame: Frame) {
    match frame {
      Frame::GpuTextures {
        generation,
        width,
        height,
        handles,
      } => {
        self.retire_ring();
        self.generation = generation;
        self.current_slot = None;
        for handle in handles {
          gpui_windows::register_external_texture(handle);
          match marker_image(width, height, handle) {
            Some(image) => self.ring.push((handle, image)),
            None => gpui_windows::unregister_external_texture(handle),
          }
        }
      }
      Frame::GpuFrame { generation, index }
        if generation == self.generation && index < self.ring.len() =>
      {
        self.current_slot = Some(index);
      }
      Frame::GpuFrame { .. } => {}
      Frame::Cpu {
        width,
        height,
        pixels,
      } => {
        if let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) {
          let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
          if let Some(old) = self.cpu_frame.replace(image) {
            self.stale.push(old);
          }
        }
      }
    }
  }

  fn retire_ring(&mut self) {
    for (handle, image) in self.ring.drain(..) {
      gpui_windows::unregister_external_texture(handle);
      self.stale.push(image);
    }
  }

  fn release_all(&mut self, cx: &mut App) {
    self.retire_ring();
    self.stale.extend(self.cpu_frame.take());
    for image in self.stale.drain(..) {
      cx.drop_image(image, None);
    }
  }

  fn frame(&self) -> Option<Arc<RenderImage>> {
    match self.current_slot {
      Some(slot) => self.ring.get(slot).map(|(_, image)| image.clone()),
      None => self.cpu_frame.clone(),
    }
  }
}

/// An image whose bytes tell the patched atlas to sample the shared texture `handle`.
fn marker_image(width: u32, height: u32, handle: u64) -> Option<Arc<RenderImage>> {
  let mut bytes = vec![0u8; width as usize * height as usize * 4];
  if bytes.len() < 16 {
    return None;
  }
  bytes[..8].copy_from_slice(EXTERNAL_TEXTURE_MAGIC);
  bytes[8..16].copy_from_slice(&handle.to_le_bytes());
  let buffer = image::RgbaImage::from_raw(width, height, bytes)?;
  Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

impl Render for VideoView {
  fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    for image in self.stale.drain(..) {
      let _ = window.drop_image(image);
    }
    let on_resize = self.on_resize.clone();
    let frame = self.frame();
    let radii = self.corner_radii;
    div()
      .size_full()
      .rounded_tl(radii.top_left)
      .rounded_tr(radii.top_right)
      .rounded_br(radii.bottom_right)
      .rounded_bl(radii.bottom_left)
      .bg(gpui::black())
      .child(
        canvas(
          move |bounds: Bounds<Pixels>, window, _| {
            let scale = window.scale_factor();
            let w = (f32::from(bounds.size.width) * scale).round() as u32;
            let h = (f32::from(bounds.size.height) * scale).round() as u32;
            on_resize(w, h);
          },
          move |bounds, (), window, _| {
            if let Some(frame) = frame {
              let _ = window.paint_image(bounds, bounds, radii, frame, 0, false);
            }
          },
        )
        .size_full(),
      )
  }
}
