# 0031: 7TV paints and badges in the chat

- Status: accepted
- Date: 2026-11-07

## Context

The user asked for 7TV's name effects and other decorations in the chat. 7TV users wear a
**paint** (a gradient or an image through the letters of their name) and a **badge**, and some
have personal emotes. 7TV's REST gives a user's `style.paint_id` and `badge_id`, but not what
they look like, and the old `cosmetics` endpoints (`/v2/cosmetics`, `/v3/cosmetics`) answer 404.
The extension gets them from the EventAPI. Checked on 2026-11-07 with `seventv_events --cosmetics`
(150 top lives, 3 minutes, anonymous): subscribing to `cosmetic.*` and `entitlement.*` with the
condition `{ctx: "channel", platform: "TWITCH", id: <channel's Twitch id>}` brings 2107
dispatches. Each `cosmetic.create` (95 paints, 56 badges) came before the first
`entitlement.create` that named it (346 for paints and badges, each matching the user's
`style`). Entitlements carry the 7TV user with their Twitch connection. They come for users who
announce their presence in that chat, through 7TV's extension. 1238 `EMOTE_SET` entitlements and
their sets (`emote_set.create`, `.update`) came too: those are the personal emotes.

GPUI draws a glyph in one color: it has no `background-clip: text` and no text shadow
(`gpui-pre 0.3.10`, `HighlightStyle`).

## Decision

- The chat's existing EventAPI connection (ADR 0030) also subscribes to the channel's
  cosmetics (`Topic::Channel`, two subscriptions). Peeby announces no presence: it sends nothing
  but subscriptions, as before.
- Definitions are kept for the session by 7TV id. What each chatter wears is kept for the chat,
  by Twitch user id. A change goes to the panel as `ChatEvent::AuthorStyle`, which applies when
  drawing: lines already on screen take it too.
- A gradient paint (linear or radial, repeating or not) colors each letter with the gradient's
  color at the letter's middle. The letters are taken as equal cells. Image paints (about 30%)
  show only their plain `color` when they give one, else the name keeps its Twitch color
  (none of the 29 seen gave one: in practice they look like no paint).
  Shadows are not drawn. Faint colors are raised to 60% opacity and made readable, as Twitch
  colors are.
- The 7TV badge comes after the Twitch badges.
- Personal emotes are left for later: entitlements of emote sets, and updates of sets no chat
  shows, are ignored.

## Consequences

- No new connection or request. The connection now opens even for a channel without 7TV
  emotes.
- A chatter who announced their presence before the chat opened has no paint until they
  announce it again (the extension does so often: about three times each in 3 minutes).
- The look is close to 7TV's for gradients, not exact: no glow, no image through the letters.
- Revisit for personal emotes, or if GPUI gains gradient text.
