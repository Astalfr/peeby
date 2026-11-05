# 0006: Peeby's visual system, with a red accent

- Status: accepted
- Date: 2026-11-01

## Context

The first milestones used peeby's palette and shapes but none of its motion or layout details: cards had no hover state, loading was a "Chargement…" line, pages appeared abruptly and the settings and account pages were plain text. Peeby already solved these on the same GPUI version (its ADR 0010 and `docs/design.md`), with a value tween for state transitions and gpui-component skeletons.

## Decision

PeebTube follows peeby's visual system ([design.md](../design.md)): Fluent layout (Mica chrome, bordered content panel, sidebar entries with an accent pill), one accent, rounded surfaces for settings rows, skeletons cross-faded into content, card hover lift and ring, image fade-in and page fade. `motion.rs` is copied from peeby unchanged. The accent stays YouTube red (`#FF4E45`, since moved to the brand red `#FF0000`, see design.md) instead of peeby's amber, and the neutral greys stay cool rather than warm.

## Consequences

- Both apps share one look and one set of motion rules, so a fix in one carries over to the other.
- Hover animations re-render the page each frame while they run (`request_animation_frame`), which is cheap for a grid of cards but would be worth measuring on very long feeds.
- The watch page is not faded in, because its video surface is an external D3D11 texture.
