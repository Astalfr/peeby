# Logo

Peeby's logo is "Onglets": the mascot's portrait on a rounded card tilted to the left, over three
cards fanned out behind it like tabs. There is one card per space, in tab order: Twitch violet at
the back, then Jellyfin blue, then YouTube red right behind the portrait. See docs/design.md.

- `peeb.webp`: the source picture (1170×1163), fan art by **@hue_oo_** (their signature is at the
  top right of the picture and stays outside the logo's crop). Credit the artist wherever the logo
  ships, and ask for their permission before Peeby is distributed.
- `onglets.html`: the logo alone on a transparent square (`?size=` pixels, else the window).
  `?layer=portrait` draws only the portrait's card, in place, for the opening animation
  (`ui::splash`). Its `FIT` and `CENTER` are repeated in `crates/ui/src/splash.rs`: change both.
- `proposals.html`: the seven directions considered on 2026-11-02 (Orbite, Onglets, Prisme,
  Lecture, Néon, Barrette, Clin d'œil). Each one is shown fixed and "vivant": in the vivant version,
  the logo takes the accent of the open space. Open the page in a browser; it loads `peeb.webp`
  from this folder.

## Regenerating

`pwsh scripts/render-logo.ps1` screenshots `onglets.html` with headless Edge at 1024 px on a
transparent background. It then writes:

- `crates/ui/assets/logo.png` (256 px), drawn by `ui::assets::logo` in the top bar (22 px), on
  the sign-in cards (40 px) and at the end of the opening animation;
- `crates/ui/assets/logo-portrait.png` (512 px), the portrait's layer of the opening animation;
- `crates/peeby/assets/peeby.ico` (16, 20, 24, 32, 40, 48, 64, 128 and 256 px, as PNGs), the
  window and taskbar icon embedded by `build.rs`.

Headless Edge's viewport is smaller than its window (1000×932 for a 1024 window), so the script
passes `?size=1024` rather than relying on the window. It fails if the drawing touches the edge of
the square. If that happens, shrink `FIT` (in both files). At 16 px, the three cards only show as
a colored edge.
