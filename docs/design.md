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

- **Title bar** (44 px, a window drag area whose controls are occluded): a menu button (burger) that folds the sidebar, the back button, then PeebTube's mark (a red rounded play badge and the name). A video opens from the command line (`peebtube <url-or-id>`); there is no URL field.
- **Sidebar** (232 px): Accueil, Abonnements, À regarder plus tard (that playlist's page, as a sidebar page) and Playlists; Compte and Réglages are pinned at the bottom above a separator. The subscribed channels are not listed there (hundreds of them), but on their own page. Entries are 40 px, muted until hovered; the entry of the page at the bottom of the stack has a light background and a 3×16 px red pill on its left edge, so Abonnements stays lit on the channels and channel pages opened from it. Folded (burger button, saved as `sidebar_collapsed` in `settings.toml`), it is 56 px wide: each entry shows its icon alone, centered, with its label as a tooltip, and keeps its pill.
- **Content**: a panel with a 12 px top-left corner and a 1 px top and left border, over the chrome as in Windows 11 Settings. Pages have 32 px side margins.
- **Shapes**: 8 px radius for controls, 12 px for cards and large surfaces. Font: Segoe UI Variable Text, else Segoe UI, at 15 px.

## Pages

- **Feeds** (Home, subscriptions, channel): rows of 416 px cards, as many per row as the content panel holds. The page header (title, grid/list switch, "Toutes les chaînes") stays at the top while the videos scroll under it. A card shows the 16:9 thumbnail (duration badge bottom right), the channel avatar, a title of at most two lines, the channel and views · date. Cards keep their natural height and a row aligns them at the top (`h_flex` centers its children by default, which lowered cards with a one-line title). The subscriptions page has a grid/list switch and a "Toutes les chaînes" button.
- **List mode** (subscriptions only, saved as `subscriptions_layout` in `settings.toml`): one row per video as YouTube showed them until about 2023, across the whole page width. A row has a 246 px thumbnail, a two-line title at 18 px, the channel with a 20 px avatar, then views · date. On hover it gets a soft surface and a play button, and the title turns red. There is no description snippet, since the feed does not carry one.
- **Playlists** (sidebar): a grid of 320 px playlist cards. A card shows the first video's thumbnail over two narrower layers peeking out above it (a stack), a count badge with a list icon, the title and its details (privacy, last update). Hover: as video cards.
- **Playlist**: a 320 px column on the left with the cover, the title (2xl, bold), owner and count, and two buttons, **Tout lire** (red) and **Aléatoire**. On the right, the videos as numbered rows (160 px thumbnail, two-line title, channel · views · date), loading the next 100 as the end comes near. A click on a row plays the playlist from there.
- **Queue panel** (watch page of a playlist, above the related videos): a bordered card with the playlist's title, the place (`3 / 17`, `+` while more are to load), shuffle and repeat toggles, then up to 420 px of rows in play order (100 px thumbnails). The current video's row is on `accent_soft` with a red play icon instead of its number, and the panel opens scrolled to it. The player gets previous and next buttons around play/pause (keys P and N), disabled at the ends.
- **Saving to a playlist**:
  - grid cards and list rows show, on hover only, a round **Watch later** button at the thumbnail's top right: a clock, or a check when the video is in "Watch later" already (a click then removes it; the "⋮" menu entry toggles the same way) and a **⋮** button by the title. Their clicks are their own, not the card's;
  - **⋮** opens a popover with "À regarder plus tard" and "Ajouter à une playlist", which turns into the list of playlists;
  - the watch page has an **Enregistrer** button left of the votes, opening that list directly;
  - the list ("Enregistrer dans…") checks the playlists holding the video (a red box with a check), a lock marks private ones; a click adds or removes at once and undoes the check if YouTube refuses;
  - in a playlist the user can edit (their own, "Watch later"), each video of the playlist page and of the queue panel gets an **x** on hover (not the playing one), removing it from the playlist;
  - while a request runs, its "x" or "Watch later" button shows a small spinner and stays visible after the mouse leaves; a second click sends nothing;
  - a playlist's page has a reverse button (arrow-down-up, highlighted when on) next to "Tout lire" and "Aléatoire": the list and its playback go last video first, numbers counting down; saved per playlist (`reversed_playlists` in `settings.toml`). A reversed playlist loads all its pages first, with a loader and the count so far;
  - every change shows a toast, at the bottom right of the window: "Ajoutée à « … »", "Retirée de « … »", or the error.
- **Channels** (from "Toutes les chaînes"): the subscribed channels as a grid of 168 px tiles (96 px round avatar, name, subscribers), a red ring and a soft surface on hover, a filter field in the header.
- **Watch page**: the 16:9 player, the title, the channel (red on hover), the votes pill (likes | dislikes, red ratio bar, RYD attribution; it links to returnyoutubedislike.com), the description card, then the comments. The related videos are a 400 px column of compact cards that get a soft surface on hover. **Theater mode** ("Mode cinéma", a button left of fullscreen and the T key, saved as `theater` in `settings.toml`): the player spans the content panel on black, without corners, above the two columns; its height is 16:9 of the width but leaves 120 px under it for the title (at least 240 px), and mpv letterboxes the picture.
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
| Busy button, list end | `ui::loader`: a faint ring with a round-capped arc turning inside it, stretching to 270° then shrinking to 12° as it turns (Material style), drawn on a canvas from a shared clock | 1.4 s per stretch, 2 s per turn |
| Skeleton to content | `motion::crossfade` on the feeds | 300 ms |

`motion.rs` is copied from peeby. GPUI only animates elements as they appear (`with_animation`); for state changes such as hover, `motion::tween` keeps a value per element id and eases it toward its target, requesting frames until it arrives. Ids must be unique per list (`card-…`, `related-…`): two elements with one id share GPUI's element state, so their hover flags overwrite each other on every frame and the card jumps up and down. Feeds therefore drop the videos YouTube repeats (the subscriptions feed repeats several per page).

GPUI takes a single `on_hover` listener per element (a second one is a debug assertion): on cards, the hover flag and the stream prefetch share it, so the prefetch keeps playback within the 1–2 s budget.
