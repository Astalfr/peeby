# 0023: Sending to a Twitch chat

- Status: accepted (the sign-in was checked; no message has been sent yet)
- Date: 2026-11-04

## Context

The user asked, on 2026-11-04, for three things in the chat: a field to write messages, an
emote and emoji picker, and under the field the channel points on the left with a "Send" button on
the right. Until now [twitch-plan.md](../twitch-plan.md) kept sending chat messages out of scope,
as a write to the account. The chat was read anonymously (`justinfan<n>`), and that connection
cannot send anything.

Options considered:

- **IRC signed in with the web `auth-token`**: `PASS oauth:<auth-token>` and `NICK <login>`,
  which is how twitch.tv's own chat connects. It was checked on 2026-11-04 with the user's
  auth-token (`twitch_capture -- chat-login`): Twitch accepted the sign-in and the `JOIN`, and
  nothing was sent.
- **Helix `POST /chat/messages`**: it needs the `user:write:chat` scope. Peeby's Device Code Grant
  does not ask for it, and Helix is optional since [ADR 0016](0016-twitch-without-helix-sign-in.md).
- **GQL `sendChatMessage`**: it is not tried. It may meet the integrity check, and Peeby never
  routes around that ([ADR 0012](0012-twitch-channel-points.md)).

## Decision

- **A second IRC connection, signed in, that only writes** (`twitch::chat::connect_writer`, through
  `Client::chat_writer`, so the token stays inside the `twitch` crate):
  - The anonymous reader stays as it is. Twitch sends a message to every connection in the room
    except the one that wrote it, so the reader shows the viewer's own lines with their real tags
    (badges, color, emotes). There is no local echo.
  - Each text is cleaned before it leaves: line breaks become spaces, because one would start another
    IRC command; `NUL` is dropped; the text is cut at 500 `char`s, Twitch's limit; and `/me …` is sent
    as an action.
  - At most 20 messages per 30 s leave, Twitch's limit for a viewer. Going beyond it locks the
    account out of chat for 30 minutes. A message past the limit is not sent, and the chat says so.
  - A refused sign-in (`Login authentication failed`) is not tried again. The field turns off and
    says to add the token again.
  - The writer's `NOTICE`s (`msg_slowmode`, `msg_followersonly`, `msg_duplicate`, `msg_banned`…)
    show in the chat as readable lines, in English and French.
  - The token never reaches a log or an error message: if sending `PASS` fails, the error only says
    "cannot sign in".
- **The writing area is generic** (`ui::chat::ChatPanel::with_composer`), with no service type:
  - the panel emits `ChatPanelEvent::Send(text)`, and the space sends it;
  - `Composing` (`Ready`, `Waiting(why)`, `Off(why)`) says whether the viewer can write;
  - the picker has an "Emotes" tab, the space's `EmoteGroup`s with a search, and an "Emoji" tab, a
    short built-in list drawn with Segoe UI Emoji (a whole Unicode list is not worth a dependency);
  - the space's own element sits on the left under the field, the options' gear then "Send" on the
    right;
  - the gear's menu is the space's own (`ChatOption`, `ChatPanel::set_options`), and the option
    chosen comes back as `ChatPanelEvent::Option(id)`: the panel knows no service here either. No
    option, no gear. Added on 2026-11-07 for "Reload 7TV emotes" (docs/twitch.md).

  The VOD replay and YouTube lives have no composer.
- **In the Twitch space**:
  - The picker offers the channel's 7TV and BTTV emotes, then the global ones: the sets the chat
    already loads.
  - Twitch's own emotes are not offered yet. The global ones and the viewer's subscription
    emotes need a new GQL query, and its saved response comes first. Typing their code still works:
    Twitch parses it.
  - Without the auth-token, the field is off and says how to add the token.
- **The channel points move under the field.** This amends ADR 0012's "at the header's right":
  - the balance, its rewards popover (which now opens upward) and the bonus button sit under the
    field;
  - the predictions stay at the header's right.

  Both are drawn from the same `PointsView`, so the points are read, and the watch time
  reported, once.

## Consequences

- The user can chat from Peeby. Every message is a public write from their account: test sends
  need the user's agreement, as playback tests do.
- Two connections per open live while signed in. The writer also receives the room's lines and
  ignores them.
- To check on the first sent message: that the reader shows it. If Twitch stops relaying a
  message to the same account's other connections, a local echo built from `USERSTATE` would be
  needed.
- Revisit if Twitch closes IRC to the web token, or retires IRC for EventSub and Helix. Sending would
  then need the `user:write:chat` scope.
