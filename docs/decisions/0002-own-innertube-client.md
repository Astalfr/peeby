# 0002: Own InnerTube client

- Status: accepted
- Date: 2026-11-01

## Context

The YouTube Data API v3 has had no recommendations endpoint since 2016, so the Home page needs YouTube's private InnerTube API. Options:
- **RustyPipe**: last release April 2025, no Home method, and an October 2026 field test found its signature deobfuscation broken.
- **yt-dlp**: extraction only (no feeds), and one Python process per call.
- **Hand-written client**: few endpoints are needed (`browse`, `next`, `player`).

## Decision

Crate `innertube`: `reqwest` + `serde_json::Value`, parsed leniently.
- Grids (Home, subscriptions, channel tabs) and related videos all use `lockupViewModel`, so one parser covers them.
- A malformed item is skipped; it never fails a whole feed.
- Every parser is tested against a real response saved in `crates/innertube/tests/fixtures/` (public IP and signatures redacted). A new endpoint gets a fixture before its parser.
- The WEB `clientVersion` is one constant, refreshed at startup from the page's ytcfg.

## Consequences

- No dependency on an abandoned crate, and parsers can be fixed quickly when YouTube changes a response.
- We own the maintenance: when YouTube changes a renderer, a fixture and a parser change.
- Comments arrive as `frameworkUpdates.entityBatchUpdate.mutations[].payload.commentEntityPayload`, not as renderers (see `comments.json`).
