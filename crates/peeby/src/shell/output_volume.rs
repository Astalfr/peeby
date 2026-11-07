//! The volume slider of the audio output menu: Windows' own volume of the output the players
//! play to (the one picked in the menu, or Windows' default), as Windows' slider sets it, for
//! every app. Read when the menu opens, written while the slider moves, both off the UI thread.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  App, Bounds, Context, DispatchPhase, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
  Pixels, ScrollEvent, Task, WeakEntity, canvas, div, px, relative,
};
use gpui_component::{ActiveTheme, h_flex};
use ui::assets::Icon;
use ui::{tf, theme};

use super::Shell;
use crate::desktop::sys::{self, OutputVolume};
use crate::settings::Settings;

/// The wheel's step over the slider, as over the player's.
const WHEEL_STEP: f32 = 0.02;

/// The slider's state, kept by the shell.
pub struct VolumeSlider {
  /// The output's volume, as last read or set; `None` before it is read, or when it cannot be.
  volume: Option<OutputVolume>,
  /// The output `volume` belongs to (`[player] audio_device`): another one picked since shows
  /// nothing until it is read.
  device: Option<String>,
  reading: Task<()>,
  /// The left button went down on the slider and is still held.
  dragging: bool,
  /// Where the track was drawn, for the mouse's position on it.
  bounds: Rc<Cell<Bounds<Pixels>>>,
  /// A volume waits for the writer, which sends only the last one.
  pending: bool,
  writing: bool,
  writer: Task<()>,
}

impl Default for VolumeSlider {
  fn default() -> Self {
    Self {
      volume: None,
      device: None,
      reading: Task::ready(()),
      dragging: false,
      bounds: Rc::default(),
      pending: false,
      writing: false,
      writer: Task::ready(()),
    }
  }
}

impl Shell {
  /// Reads the selected output's volume again (the menu opened: it may have changed in
  /// Windows, or the output with it).
  pub(super) fn read_output_volume(&mut self, cx: &mut Context<Self>) {
    let device = Settings::get(cx).player.audio_device.clone();
    let device_for_read = device.clone();
    let reading = cx
      .background_executor()
      .spawn(async move { sys::output_volume(device.as_deref()) });
    self.volume_slider.reading = cx.spawn(async move |this, cx| {
      let read_for = device_for_read;
      let volume = match reading.await {
        Ok(volume) => Some(volume),
        Err(e) => {
          tracing::warn!("cannot read the audio output's volume: {e}");
          None
        }
      };
      this
        .update(cx, |this, cx| {
          // A drag started meanwhile is newer than what was read.
          if !this.volume_slider.dragging && !this.volume_slider.pending {
            this.volume_slider.volume = volume;
            this.volume_slider.device = read_for;
            cx.notify();
          }
        })
        .ok();
    });
  }

  /// Sets the output's volume; moving the slider unmutes it, as Windows' slider does.
  fn set_output_volume(&mut self, volume: OutputVolume, cx: &mut Context<Self>) {
    let slider = &mut self.volume_slider;
    if slider.volume == Some(volume) {
      return;
    }
    slider.volume = Some(volume);
    slider.device = Settings::get(cx).player.audio_device.clone();
    slider.pending = true;
    // Cancels a read under way, which would bring the old volume back.
    slider.reading = Task::ready(());
    cx.notify();
    if slider.writing {
      return;
    }
    slider.writing = true;
    // One write at a time, the latest volume each: a drag moves faster than the audio service
    // answers, and writes on several threads could land out of order.
    slider.writer = cx.spawn(async move |this, cx| {
      loop {
        let next = this
          .update(cx, |this, cx| {
            let slider = &mut this.volume_slider;
            match slider.volume.filter(|_| slider.pending) {
              Some(volume) => {
                slider.pending = false;
                Some((Settings::get(cx).player.audio_device.clone(), volume))
              }
              None => {
                slider.writing = false;
                None
              }
            }
          })
          .ok()
          .flatten();
        let Some((device, volume)) = next else {
          break;
        };
        let written = cx
          .background_executor()
          .spawn(async move { sys::set_output_volume(device.as_deref(), volume) })
          .await;
        if let Err(e) = written {
          tracing::warn!("cannot set the audio output's volume: {e}");
        }
      }
    });
  }

  /// Volume from a mouse position on the track: left is 0 %, right 100 %.
  fn set_output_volume_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
    let bounds = self.volume_slider.bounds.get();
    let width = f32::from(bounds.size.width);
    if width > 0.0 {
      let fraction = (f32::from(x - bounds.origin.x) / width).clamp(0.0, 1.0);
      let level = (fraction * 100.0).round() / 100.0;
      self.set_output_volume(
        OutputVolume {
          level,
          muted: false,
        },
        cx,
      );
    }
  }

  /// The wheel over the slider: [`WHEEL_STEP`] up or down, unmuted.
  fn step_output_volume(&mut self, up: bool, cx: &mut Context<Self>) {
    if let Some(volume) = self.current_output_volume(cx) {
      let step = if up { WHEEL_STEP } else { -WHEEL_STEP };
      let level = ((volume.level + step).clamp(0.0, 1.0) * 100.0).round() / 100.0;
      self.set_output_volume(
        OutputVolume {
          level,
          muted: false,
        },
        cx,
      );
    }
  }

  /// The volume shown, if it belongs to the output selected now.
  fn current_output_volume(&self, cx: &App) -> Option<OutputVolume> {
    let slider = &self.volume_slider;
    slider
      .volume
      .filter(|_| slider.device == Settings::get(cx).player.audio_device)
  }

  fn toggle_output_mute(&mut self, cx: &mut Context<Self>) {
    if let Some(volume) = self.current_output_volume(cx) {
      self.set_output_volume(
        OutputVolume {
          muted: !volume.muted,
          ..volume
        },
        cx,
      );
    }
  }
}

/// The slider row of the menu: a mute button, the track, the percentage. Built like the
/// player's volume slider; nothing while the volume is unknown.
pub(super) fn slider_row(shell: &WeakEntity<Shell>, cx: &App) -> Option<impl IntoElement + use<>> {
  let owner = shell.upgrade()?.read(cx);
  let volume = owner.current_output_volume(cx)?;
  let state = &owner.volume_slider;
  let (dragging, bounds_cell) = (state.dragging, state.bounds.clone());
  let level = if volume.muted { 0.0 } else { volume.level };
  let icon = if volume.muted || level == 0.0 {
    Icon::Muted
  } else if level < 0.5 {
    Icon::VolumeLow
  } else {
    Icon::Volume
  };
  let primary = cx.theme().primary;
  let mute = {
    let shell = shell.clone();
    div()
      .id("output-mute")
      .flex_none()
      .p_1()
      .rounded(theme::RADIUS)
      .cursor_pointer()
      .hover(|s| s.bg(theme::surface_hover()))
      .on_click(move |_, _, cx| {
        shell
          .update(cx, |shell, cx| shell.toggle_output_mute(cx))
          .ok();
      })
      .child(gpui_component::Icon::new(icon).size(px(16.)))
  };
  let track = div()
    .relative()
    .w_full()
    .h(px(4.))
    .rounded_full()
    .bg(theme::border())
    .child(
      div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(relative(level))
        .min_w(px(4.))
        .rounded_full()
        .bg(primary),
    )
    .child(
      div()
        .absolute()
        .left(relative(level))
        .top(px(-5.))
        .ml(px(-7.))
        .size(px(14.))
        .rounded_full()
        .bg(primary)
        .border_2()
        .border_color(gpui::white()),
    )
    .child({
      let shell = shell.clone();
      canvas(
        move |bounds, _, _| bounds_cell.set(bounds),
        move |_, _, window, _| {
          if dragging {
            listen_to_drag(&shell, window);
          }
        },
      )
      .absolute()
      .inset_0()
    });
  let slider = {
    let (shell, wheel_shell) = (shell.clone(), shell.clone());
    // Room for the knob at both ends of the track, inside the area that takes the clicks.
    div()
      .id("output-volume")
      .flex_1()
      .min_w_0()
      .h(px(28.))
      .flex()
      .items_center()
      .px(px(8.))
      .cursor_pointer()
      .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
        shell
          .update(cx, |shell, cx| {
            shell.volume_slider.dragging = true;
            // Paints the canvas that follows the drag, even if the press kept the volume.
            cx.notify();
            shell.set_output_volume_at(event.position.x, cx);
          })
          .ok();
      })
      .on_scroll(move |event: &ScrollEvent, window, cx| {
        // The menu has nothing to scroll, nor the page under it to get.
        cx.stop_propagation();
        let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
        if dy.abs() > 0.5 {
          wheel_shell
            .update(cx, |shell, cx| shell.step_output_volume(dy > 0.0, cx))
            .ok();
        }
      })
      .child(track)
  };
  let percent = div()
    .flex_none()
    .w(px(36.))
    .text_right()
    .text_sm()
    .text_color(theme::muted())
    .child(tf!("{:.0}%", "{:.0} %", volume.level * 100.0));
  Some(
    h_flex()
      .w_full()
      .px_1()
      .gap_1()
      .items_center()
      .child(mute)
      .child(slider)
      .child(percent),
  )
}

/// Follows the slider's drag over the whole window until the release. Registered at each paint
/// while the drag lasts, as GPUI drops window listeners every frame.
fn listen_to_drag(shell: &WeakEntity<Shell>, window: &mut gpui::Window) {
  let move_shell = shell.clone();
  window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture {
      move_shell
        .update(cx, |shell, cx| {
          if event.pressed_button == Some(MouseButton::Left) {
            shell.set_output_volume_at(event.position.x, cx);
          } else {
            // The button was released where no mouse up reached the window.
            shell.volume_slider.dragging = false;
            cx.notify();
          }
        })
        .ok();
    }
  });
  let up_shell = shell.clone();
  window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
      up_shell
        .update(cx, |shell, cx| {
          shell.volume_slider.dragging = false;
          cx.notify();
        })
        .ok();
    }
  });
}
