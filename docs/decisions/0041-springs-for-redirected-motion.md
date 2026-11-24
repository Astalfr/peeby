# 0041: Springs for motion that changes direction

- Status: accepted
- Date: 2026-11-24

## Context

`motion::tween` eases a value toward its target with `ease_out_cubic` over a fixed duration.
When the target changes mid-way, it starts a new ease from the current value at zero speed:
a card left halfway through its hover rise stops, then starts back; the menu button clicked
again mid-fold stops the sidebar dead before reversing it. The tab bar's slides and the Shorts'
slide had the same problem in their own code: a tab passed twice restarted its ease, and a
second Short asked for during a slide jumped the Short that was coming in back to its place.

Apple's "fluid interfaces" and SwiftUI handle this with springs, which keep their speed when
they are redirected. Fluent's fixed durations stay the reference for the time things take
(design.md, "Motion").

## Decision

- `ui::motion::Spring`: a damped spring with SwiftUI's parameters, `response` (the period
  without damping, about the time it takes to get there when critically damped) and `damping`
  (the damping ratio; `motion::CRITICAL`, 1, means no bounce). Its motion is solved in closed
  form from its last change (critically, under- and overdamped cases), so it is the same at any
  frame rate and testable without a clock. A new target (`retarget`) or a shift of its value
  (`shift`) keeps its value and speed. It rests once its distance and its speed over a radian
  are under 0.001 (in the value's units), and then returns its target exactly.
- `motion::spring(id, target, response, damping, window, cx)`: the keyed-state version of
  `tween`, for render code. It starts at rest on its first target, and with reduced motion it
  is at its target at once.
- Used, critically damped, with the old durations as their response, where values are
  redirected often: the card hover (`card::Hover`), the sidebar's and the theater chat's folding,
  the tabs sliding while one is dragged (each slide added to the one under way), the Shorts'
  slide (a new move starts from where the Shorts are drawn). Values used as opacities or
  fractions are clamped to [0, 1].
- `tween` stays for one-way animations and for those that need an exact duration (fades,
  press release, poll bars, the splash).

## Consequences

- Reversing a hover or a fold no longer stops it: the motion bends back.
- A spring's end is not fixed: a critically damped spring of response R is within 1 % after
  about R and asks for frames a little longer, until it rests (about 3 R for a unit move).
  Code that waited a fixed duration (the Shorts' timer that drops the Short leaving) now ends
  the motion when the spring rests, with a longer timer kept only for a page no longer drawn.
- With reduced motion, the Shorts no longer slide (they did before).
- Revisit if a bouncy spring is wanted somewhere (under 1 damping) or if GPUI gains transitions
  of its own.
