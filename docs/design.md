# Design

PeebTube looks like a Windows 11 app with a neutral dark theme. It takes peeby's visual system as it is (Fluent shapes, Mica chrome, a single accent, discreet motion), with YouTube's red as the accent ([ADR 0006](decisions/0006-peeby-visual-system.md)). The values live in `crates/peebtube/src/theme.rs`. Pages use `theme::…` instead of hard-coded colors, and gpui-component widgets get the same palette through `theme::apply`. Peeby's own description is `D:\DEV\peeby\docs\design.md`.

## Palette

| Token | Value | Use |
|---|---|---|
| `base` | `#0B0B0C` | Window chrome without Mica |
| Mica veil | `#0B0B0C` at 55 % | Over Mica, so the chrome stays dark |
| `background` | `#121214` | Content panel, pages |
| `surface` | `#1C1C1F` | Cards, settings rows, description, inputs |
| `surface_hover` | `#27272B` | Hovered surfaces, skeletons |
| `border` | `#2F2F34` | Borders, separators |
| `foreground` | `#F2F2F3` | Text |
| `muted` | `#A1A1AA` | Secondary text |
| `subtle` | `#71717A` | Section labels, fine print |
| `accent` | `#FF4E45` | Primary buttons, progress, selection, play buttons, active item |
| `accent_hover` | `#FF6B63` | Hovered accent elements and titles |
| `accent_soft` | red at 14 % | Notice discs, step numbers, selected rows |

One accent: red marks what can be acted on or what is selected. Everything else stays neutral, so the thumbnails carry the color.

## Window and layout

- **Title bar** (44 px, a window drag area whose controls are occluded): back button, PeebTube's mark (a red rounded play badge and the name), then the "open a URL or id" field centered, with a search icon.
- **Sidebar** (232 px): Accueil and Abonnements; Compte and Réglages are pinned at the bottom above a separator. The subscribed channels are not listed there (hundreds of them), but on their own page. Entries are 40 px, muted until hovered; the entry of the page at the bottom of the stack has a light background and a 3×16 px red pill on its left edge, so Abonnements stays lit on the channels and channel pages opened from it.
- **Content**: a panel with a 12 px top-left corner and a 1 px top and left border, over the chrome as in Windows 11 Settings. Pages have 32 px side margins.
- **Shapes**: 8 px radius for controls, 12 px for cards and large surfaces. Font: Segoe UI Variable Text, else Segoe UI, at 15 px.

## Pages

- **Feeds** (Home, subscriptions, channel): a wrapping grid of 320 px cards. A card shows the 16:9 thumbnail (duration badge bottom right), the channel avatar, the title and the channel and views · date. The title always takes two 20 px lines and the metadata two 18 px lines, so every card has the same height and rows line up whatever the text. The subscriptions page has a "Toutes les chaînes" button.
- **Channels** (from "Toutes les chaînes"): the subscribed channels as a grid of 168 px tiles (96 px round avatar, name, subscribers), a red ring and a soft surface on hover, a filter field in the header.
- **Watch page**: the 16:9 player, the title, the channel (red on hover), the votes pill (likes | dislikes, red ratio bar, RYD attribution; it links to returnyoutubedislike.com), the description card, then the comments. The related videos are a 400 px column of compact cards that get a soft surface on hover.
- **Settings**: sections of rounded rows on `surface`, title and description on the left, small choice chips on the right (the selected one red).
- **Account**: a status card (icon, state, cookie file, sign-out), then the export steps, numbered on red discs, and the import button.
- **Empty and error states**: centered, an icon on a soft red disc, a title, a body and the action (import cookies, retry).
- GPUI clips children to rectangles, so the radius is set on the image itself (`image_fill(…, radius, …)`), not only on its container. That is also why cards do not zoom on hover.

## Motion

Discreet and short; nothing loops except skeletons and spinners.

| What | How | Duration |
|---|---|---|
| Card hover | Its content rises 4 px inside a box that stays in place (a moving box could slip out from under the pointer and loop), a red ring and a play button fade in, the title turns red, shadow | 150 ms |
| Related card hover | A soft surface and a play button fade in, the title turns red | 150 ms |
| Image load | The image fades in over the flat surface | 200 ms |
| Page change | Fades in and rises 8 px (not the watch page: the video surface is an external texture, and leaving fullscreen would replay it) | 200 ms |
| Loading | gpui-component skeletons shaped like the content (grid cards, title and channel, related cards), a spinner at the end of a list | — |
| Skeleton to content | `motion::crossfade` on the feeds | 300 ms |

`motion.rs` is copied from peeby. GPUI only animates elements as they appear (`with_animation`); for state changes such as hover, `motion::tween` keeps a value per element id and eases it toward its target, requesting frames until it arrives. Ids must be unique per list (`card-…`, `related-…`): two elements with one id share GPUI's element state, so their hover flags overwrite each other on every frame and the card jumps up and down. Feeds therefore drop the videos YouTube repeats (the subscriptions feed repeats several per page).

GPUI takes a single `on_hover` listener per element (a second one is a debug assertion): on cards, the hover flag and the stream prefetch share it, so the prefetch keeps playback within the 1–2 s budget.
