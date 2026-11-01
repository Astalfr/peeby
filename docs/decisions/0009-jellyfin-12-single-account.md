# 0009: Jellyfin 12.0+, one server, one account

- Status: accepted
- Date: 2026-11-01

## Context

The Jellyfin space takes over peeby's Jellyfin client and its `jellyfin-api` crate, copied here with its
fixture tests (step 5 of [refactor-plan.md](../refactor-plan.md)). Peeby decided its API surface and session
model in its ADR 0005 ([archived](peeby/0005-jellyfin-12-single-account.md)): the Jellyfin API changed paths
between 10.8, 10.9 and later versions, and supporting several versions, servers or accounts adds code paths,
settings and screens without helping the main goal.

## Decision

Peeby keeps that decision for its Jellyfin space:

- The minimum server version is 12.0; the login page rejects older servers (`GET /System/Info/Public`).
- One server and one account; switching means signing out.
- The access token lives in the Windows Credential Manager (`keyring`), the server URL, user id and device id in
  `settings.toml`. A token is small, unlike YouTube's cookie jar (ADR 0004). It never appears in a URL or a log.
- The reference spec is [openapi-12.4.0.json](../reference/openapi-12.4.0.json); only endpoints that also exist
  in 12.0 are used.

## Consequences

- One API surface, tested on saved responses (`crates/jellyfin-api/tests/fixtures/`) and, by hand, against a
  real server (`tests/live.rs`, ignored by default).
- The former peeby's session is not migrated (ADR 0007): the user signs in again.
- Several servers would need a new ADR and a session model per server.
