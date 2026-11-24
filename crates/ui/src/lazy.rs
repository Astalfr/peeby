//! Items of a long scrolling page that are far out of view are not built (docs/design.md,
//! "Long pages"). A page that is not a `list` (the watch page: details, comments, related
//! videos in columns) would otherwise build, lay out and paint every comment at every frame of a
//! scroll. [`Lazy::item`] measures each item where it is drawn; one more than a viewport away
//! from the view is drawn as an empty box of its last height instead, so the page keeps its
//! length and the scroll its place.
//!
//! An item's height is the one it had when last built: a change of the view's width forgets
//! them all (the text rewraps). An item changes size only through what the user does on it, in
//! view.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Pixels, ScrollHandle, SharedString, canvas, div, px};

/// The items of a page scrolled by one [`ScrollHandle`]; cheap to clone.
#[derive(Clone)]
pub struct Lazy {
  scroll: ScrollHandle,
  places: Rc<RefCell<Places>>,
}

#[derive(Default)]
struct Places {
  /// The view's width when they were measured.
  width: Pixels,
  items: HashMap<SharedString, Place>,
}

/// Where an item was at the last frame, from the top of the scrolled content.
#[derive(Clone, Copy)]
struct Place {
  top: Pixels,
  height: Pixels,
}

impl Lazy {
  /// The items of the page that `scroll` tracks.
  pub fn new(scroll: ScrollHandle) -> Self {
    Self {
      scroll,
      places: Rc::default(),
    }
  }

  /// The item `key` (unique in this page), built by `build` unless it is far out of view.
  pub fn item(
    &self,
    key: impl Into<SharedString>,
    build: impl FnOnce() -> AnyElement,
  ) -> AnyElement {
    let key = key.into();
    let view = self.scroll.bounds();
    let far = {
      let mut places = self.places.borrow_mut();
      if places.width != view.size.width {
        places.width = view.size.width;
        places.items.clear();
      }
      let height = view.size.height;
      let top = -self.scroll.offset().y;
      places.items.get(&key).copied().filter(|place| {
        height > px(0.)
          && (place.top + place.height < top - height || place.top > top + height * 2.)
      })
    };
    let content = match far {
      Some(place) => div().w_full().h(place.height).into_any_element(),
      None => build(),
    };
    let (places, scroll) = (self.places.clone(), self.scroll.clone());
    div()
      .relative()
      .child(content)
      .child(
        canvas(
          move |bounds, _, _| {
            let top = bounds.origin.y - scroll.bounds().origin.y - scroll.offset().y;
            places.borrow_mut().items.insert(
              key,
              Place {
                top,
                height: bounds.size.height,
              },
            );
          },
          |_, (), _, _| {},
        )
        .absolute()
        .inset_0(),
      )
      .into_any_element()
  }
}
