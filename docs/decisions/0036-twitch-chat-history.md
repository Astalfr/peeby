# 0036: A Twitch chat's history from recent-messages

- Status: accepted
- Date: 2026-11-11

## Context

A live's chat opened empty until someone wrote: Twitch's IRC sends no history on joining, and
twitch.tv's own history comes from a GQL query of its web client that answers only with an
integrity token. The user asked (2026-11-11, docs/features-plan.md, T1) that a chat open with
its last messages, as twitch.tv's does, and agreed to an outside service for it.

[robotty's recent-messages](https://recent-messages.robotty.de) is the service Chatterino and
other chat clients use: it stays joined to channels it was asked about and keeps their last 800
lines, answering `GET /api/v2/recent-messages/<login>` anonymously with the raw IRC lines Twitch
sent (tags included, `historical=1` and `rm-received-ts` added), so they parse as live lines.

## Decision

- `twitch::Client::recent_chat` asks for the channel's last 100 lines; `chat::parse_recent`
  parses them with the live parser and keeps messages, notices and their removals. Its answer is
  a fixture (`recent_messages.json`), the chatters renamed as every chat fixture.
- A live's chat asks for it at once, beside the connection, and waits for it 3 s at most: it
  goes first among the lines held while the channel's emotes load (so it shows with them), then
  a notice line ("Au-dessus, les messages d'avant votre arrivée") and the live lines. A failure
  or a late answer leaves the chat as before (a log line).
- The setting "Messages précédents" (Twitch's part, Chat; on by default) turns it off: the
  service learns which channel's chat opens, said in its description.

## Consequences

- One more outside service, anonymous, told the logins of the chats opened.
- A channel nobody asked the service about before has no history the first time (the service
  joins it then).
- The replay of a VOD's chat is not concerned (it has its own source).
