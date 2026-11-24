# Teasers and hotspots

Links a space places over the picture of the shared player (`crates/ui/src/player/hotspots.rs`,
[ADR 0042](../decisions/0042-cards-and-end-screens.md)). The player knows no service: each item
comes with its picture, its text and what a click does (`OnPick`).

## Teasers (YouTube's cards)

`PlayerView::set_teasers`. A `Teaser` shows at the top right from its time `at` for
`TEASER_FOR` (6 s), the latest one when two overlap: a picture (16:9, or round for a channel),
its text and a line under it. As the "now playing" card at the top left, it slides in from
beyond the player's right edge and back there when its time is up (`theme::SLOW`, easing out
then in; at once with reduced motion), clipped at that edge, and moves under the title bar or the top bar with the controls' fade (its length, its
curve: `under_top_bar`, shared with the "now playing" card) while they show. A click follows
it. None on a live, in picture-in-picture.

## Hotspots (YouTube's end screens)

`PlayerView::set_hotspots`. A `Hotspot` shows from `start` to `end`, placed and sized as shares
of the picture: mpv draws it letterboxed inside the player, so its box is the player's less the
bars, from the picture's aspect (`video-params/aspect`, read from mpv once when the first
hotspot shows; 16:9 until known). Its height is its width over its `aspect_ratio`. A video is a
rounded thumbnail with its length in a corner; a channel or a site is round. Its title (and
subtitle) shows while the pointer is over it: over a video's top edge, under a round one. A
click follows it. They are drawn under the skip button, the "up next" card and the controls,
which cover them where they overlap. None on a live, in picture-in-picture.
