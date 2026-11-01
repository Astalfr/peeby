# 0010: Fluent window chrome with a warm amber theme

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-11-01

## Context

Until phase 4, the app used gpui-component's default dark theme: grey surfaces, white primary buttons, no accent, Windows' own title bar, a text-only sidebar and no motion. The user asked for a visual identity and chose:
- an **amber / coral** palette (over violet, blue, or monochrome with colors taken from the artwork);
- a **Fluent / Windows 11** style (over immersive streaming-app, minimalist, or dense layouts);
- **discreet** animations;
- Segoe UI Variable;
- a hero on the home page showing what to resume;
- a detailed episode list;
- a collapsible sidebar.

Constraints:
- GPUI has no transitions between states, only animations of appearing elements.
- gpui-component's `motion` module is not re-exported, and adding `gpui-base` as a direct dependency needs approval.
- Mica follows the system light/dark setting.

## Decision

- A `theme` module holds the palette and the shapes, and applies them to gpui-component's theme. The details are in [design.md](../../design.md).
- The app draws its own title bar (gpui-component `TitleBar`).
- The window uses **Mica**, only when Windows is in dark mode, under a dark veil. Otherwise the chrome is a solid color.
- A small `motion` module (`tween` built on keyed element state and animation frames) provides hover and fade transitions without a new dependency.

## Consequences

- Every page uses `theme::` tokens. A palette change is one file.
- The title bar is app code. The player embeds it in its top bar so the window stays movable during playback.
- Hover transitions cost a few frames of re-rendering per hovered card, which is negligible.
- If gpui-component re-exports its motion API, or `gpui-base` gets approved, `motion::tween` can be replaced by `transition`.
