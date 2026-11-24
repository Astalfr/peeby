//! Text the mouse can select and Ctrl+C copy, as in a browser: comments, titles, chat lines.
//!
//! It takes part in the window's selection of `gpui-base` (the `TextSelectionLayer` that
//! gpui-component's `Root` draws over each window, and its Ctrl+C): a drag from anywhere selects
//! the text it passes over, across elements, and a click elsewhere clears it.
//!
//! [`SelectableText`] wraps a text element (a [`StyledText`], or an [`gpui::InteractiveText`] built on
//! one, whose links still answer clicks) and paints the selected part of it behind the text.
//! Alone, it is a selectable document of its own; in a [`SelectionGroup`] (a chat line, whose
//! words are separate elements), its text joins the group's, so that copying a line gives its
//! words in a row rather than one per line.
//!
//! Copying several documents joins them with line breaks, in reading order: top to bottom, then
//! left to right, from where they are on screen.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
  AnyElement, App, BorderStyle, Bounds, Corners, CursorStyle, Edges, Element, ElementId,
  GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
  PaintQuad, Pixels, Point, SharedString, StyledText, TextLayout, Window, transparent_black,
};
use gpui_base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};

use crate::theme;

/// The selection's background, the one gpui-component's own text fields use.
fn selection_color() -> Hsla {
  theme::accent().opacity(0.3)
}

/// Whether the window has text selected: a click that ends a drag over a link or a name selects,
/// and should not also follow it.
pub fn has_selection(window: &mut Window, cx: &mut App) -> bool {
  !TextSelection::selected_text(window, cx).is_empty()
}

/// Ctrl+C: copies the window's selected text, if any; whether there was some. gpui-component's
/// own binding answers only while the focus is inside its `Root` (not after a click that took it
/// away, nor in a text field, which keeps the key), so the shell calls this on every Ctrl+C. A
/// click anywhere clears the window's selection: a field's own selection is never shadowed.
pub fn copy(window: &mut Window, cx: &mut App) -> bool {
  let text = TextSelection::selected_text(window, cx);
  let text = text.trim();
  if text.is_empty() {
    return false;
  }
  cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_owned()));
  true
}

/// Reading order of a document at `origin` (window coordinates): rows of pixels first, then
/// columns, for the order of copied documents.
fn reading_order(origin: Point<Pixels>) -> u64 {
  let top = f32::from(origin.y).max(0.) as u64;
  let left = f32::from(origin.x).max(0.) as u64;
  (top << 32) | left
}

/// The handle of the document whose element is `global_id`, kept from frame to frame.
fn retained_handle(
  global_id: Option<&GlobalElementId>,
  text: &SharedString,
  window: &mut Window,
  cx: &mut App,
) -> Option<TextSelectionHandle> {
  let global_id = global_id?;
  Some(
    window.with_element_state(global_id, |retained: Option<TextSelectionHandle>, _| {
      let handle = retained.unwrap_or_else(|| TextSelectionHandle::new(text.to_string(), cx));
      (handle.clone(), handle)
    }),
  )
}

/// Registers a document for this frame: `hitbox` where a drag can start in it, `runs` its text.
fn register(
  handle: &TextSelectionHandle,
  hitbox: &Hitbox,
  bounds: Bounds<Pixels>,
  runs: &[TextSelectionRun],
  window: &mut Window,
  cx: &mut App,
) {
  let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
    .with_document_order(reading_order(bounds.origin))
    .with_text_bounds(runs.iter().map(TextSelectionRun::bounds).collect())
    .with_rendered_element(handle, window, cx);
  handle.register(registration, window, cx);
}

/// The selected byte ranges of `runs`, in their order; refreshes the window when the copied text
/// changes (as `gpui_base::SelectableText` does).
fn project(
  handle: &TextSelectionHandle,
  runs: &[TextSelectionRun],
  window: &mut Window,
  cx: &mut App,
) -> Vec<Option<Range<usize>>> {
  let before = TextSelection::selected_text(window, cx);
  let projection = handle.update_runs(runs, cx);
  if before != TextSelection::selected_text(window, cx) {
    window.refresh();
  }
  projection.ranges().to_vec()
}

/// Paints the selection of `range` in `layout`: one band per row it spans.
fn paint_selection(layout: &TextLayout, range: Range<usize>, window: &mut Window) {
  let (Some(start), Some(end)) = (
    layout.position_for_index(range.start),
    layout.position_for_index(range.end),
  ) else {
    return;
  };
  let bounds = layout.bounds();
  let line = layout.line_height();
  let mut bands = Vec::new();
  if start.y == end.y {
    bands.push(Bounds::from_corners(start, Point::new(end.x, end.y + line)));
  } else {
    bands.push(Bounds::from_corners(
      start,
      Point::new(bounds.right(), start.y + line),
    ));
    if end.y > start.y + line {
      bands.push(Bounds::from_corners(
        Point::new(bounds.left(), start.y + line),
        Point::new(bounds.right(), end.y),
      ));
    }
    bands.push(Bounds::from_corners(
      Point::new(bounds.left(), end.y),
      Point::new(end.x, end.y + line),
    ));
  }
  for band in bands {
    window.paint_quad(PaintQuad {
      bounds: band,
      background: selection_color().into(),
      corner_radii: Corners::default(),
      border_widths: Edges::default(),
      border_color: transparent_black(),
      border_style: BorderStyle::default(),
    });
  }
}

/// The documents that several [`SelectableText`]s form together (a chat line): give the same group
/// to each of them and wrap their common parent with [`SelectionGroup::wrap`]. Make a new one at
/// each render.
#[derive(Clone, Default)]
pub struct SelectionGroup(Rc<GroupState>);

#[derive(Default)]
struct GroupState {
  /// The members' text as they were laid out this frame, in their order.
  runs: RefCell<Vec<TextSelectionRun>>,
  /// The selected part of each run, once the group has projected the selection.
  ranges: RefCell<Vec<Option<Range<usize>>>>,
}

impl SelectionGroup {
  /// A group with no member yet.
  pub fn new() -> Self {
    Self::default()
  }

  /// `element`, the parent of the group's members, as one selectable document. `id` is stable
  /// from frame to frame (the line's) and unique among its siblings.
  pub fn wrap(&self, id: impl Into<ElementId>, element: impl IntoElement) -> GroupElement {
    GroupElement {
      id: id.into(),
      group: self.clone(),
      child: element.into_any_element(),
    }
  }
}

/// The element of a [`SelectionGroup`] (see [`SelectionGroup::wrap`]).
pub struct GroupElement {
  id: ElementId,
  group: SelectionGroup,
  child: AnyElement,
}

impl IntoElement for GroupElement {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for GroupElement {
  type RequestLayoutState = Option<TextSelectionHandle>;
  type PrepaintState = Option<Hitbox>;

  fn id(&self) -> Option<ElementId> {
    Some(self.id.clone())
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    global_id: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let handle = retained_handle(global_id, &SharedString::default(), window, cx);
    (self.child.request_layout(window, cx), handle)
  }

  fn prepaint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    handle: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    self.group.0.runs.borrow_mut().clear();
    // The members add their runs as they are laid out.
    self.child.prepaint(window, cx);
    let handle = handle.as_ref()?;
    let runs = self.group.0.runs.borrow().clone();
    if runs.is_empty() {
      return None;
    }
    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    register(handle, &hitbox, bounds, &runs, window, cx);
    Some(hitbox)
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    _: Bounds<Pixels>,
    handle: &mut Self::RequestLayoutState,
    hitbox: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    if let (Some(handle), Some(hitbox)) = (handle.as_ref(), hitbox.as_ref()) {
      window.set_cursor_style(CursorStyle::IBeam, hitbox);
      let runs = self.group.0.runs.borrow().clone();
      *self.group.0.ranges.borrow_mut() = project(handle, &runs, window, cx);
    }
    // The members paint their part of the selection behind their text.
    self.child.paint(window, cx);
  }
}

/// A selectable text element (see the module's documentation).
pub struct SelectableText {
  id: ElementId,
  text: SharedString,
  layout: TextLayout,
  child: AnyElement,
  group: Option<SelectionGroup>,
  /// This member's run in its group, this frame.
  index: Rc<Cell<Option<usize>>>,
}

impl SelectableText {
  /// `text`, plain, in the parent's text style.
  pub fn text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
    let text = text.into();
    Self::styled(id, text.clone(), StyledText::new(text))
  }

  /// `styled`, whose text is `text` (with its highlights, say).
  pub fn styled(id: impl Into<ElementId>, text: SharedString, styled: StyledText) -> Self {
    let layout = styled.layout().clone();
    Self::wrapping(id, text, layout, styled)
  }

  /// `element`, which draws `text` with `layout` (an `InteractiveText` around a `StyledText`,
  /// whose layout was taken before: `StyledText::layout`).
  pub fn wrapping(
    id: impl Into<ElementId>,
    text: SharedString,
    layout: TextLayout,
    element: impl IntoElement,
  ) -> Self {
    Self {
      id: id.into(),
      text,
      layout,
      child: element.into_any_element(),
      group: None,
      index: Rc::default(),
    }
  }

  /// Joins `group`'s document instead of being one of its own.
  pub fn in_group(mut self, group: &SelectionGroup) -> Self {
    self.group = Some(group.clone());
    self
  }

  fn run(&self) -> TextSelectionRun {
    TextSelectionRun::new(self.text.clone(), self.layout.clone(), self.layout.bounds())
  }
}

impl IntoElement for SelectableText {
  type Element = Self;

  fn into_element(self) -> Self::Element {
    self
  }
}

impl Element for SelectableText {
  type RequestLayoutState = Option<TextSelectionHandle>;
  type PrepaintState = Option<Hitbox>;

  fn id(&self) -> Option<ElementId> {
    Some(self.id.clone())
  }

  fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
    None
  }

  fn request_layout(
    &mut self,
    global_id: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
  ) -> (LayoutId, Self::RequestLayoutState) {
    let handle = match self.group {
      Some(_) => None,
      None => retained_handle(global_id, &self.text, window, cx),
    };
    (self.child.request_layout(window, cx), handle)
  }

  fn prepaint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    bounds: Bounds<Pixels>,
    handle: &mut Self::RequestLayoutState,
    window: &mut Window,
    cx: &mut App,
  ) -> Self::PrepaintState {
    self.child.prepaint(window, cx);
    // A run must be the text its layout was made from (an empty or a clipped one is not).
    if self.text.is_empty() || self.layout.len() != self.text.len() {
      self.index.set(None);
      return None;
    }
    if let Some(group) = &self.group {
      let mut runs = group.0.runs.borrow_mut();
      self.index.set(Some(runs.len()));
      runs.push(self.run());
      return None;
    }
    let handle = handle.as_ref()?;
    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    register(handle, &hitbox, bounds, &[self.run()], window, cx);
    Some(hitbox)
  }

  fn paint(
    &mut self,
    _: Option<&GlobalElementId>,
    _: Option<&InspectorElementId>,
    _: Bounds<Pixels>,
    handle: &mut Self::RequestLayoutState,
    hitbox: &mut Self::PrepaintState,
    window: &mut Window,
    cx: &mut App,
  ) {
    let range = match (&self.group, handle.as_ref(), hitbox.as_ref()) {
      (Some(group), _, _) => self
        .index
        .get()
        .and_then(|i| group.0.ranges.borrow().get(i).cloned().flatten()),
      (None, Some(handle), Some(hitbox)) => {
        // Set before the text paints: a link's pointer, set by the text, wins over it.
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        project(handle, &[self.run()], window, cx)
          .into_iter()
          .next()
          .flatten()
      }
      _ => None,
    };
    if let Some(range) = range.filter(|r| !r.is_empty()) {
      paint_selection(&self.layout, range, window);
    }
    self.child.paint(window, cx);
  }
}
