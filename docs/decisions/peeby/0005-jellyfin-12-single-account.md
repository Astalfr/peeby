# 0005: Jellyfin 12.0+, one server, one account

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

The Jellyfin API changed paths between 10.8, 10.9 and later versions (for example `/Users/{userId}/Items/...` → `/UserItems/...`). Supporting several versions, servers or accounts adds code paths, settings and screens without helping the main goal.

## Decision

- The minimum server version is 12.0. The login screen rejects older servers, based on `GET /System/Info/Public`.
- One server and one account per installation. Switching means logging out.

## Consequences

- One API surface. The reference spec is [openapi-12.4.0.json](../../reference/openapi-12.4.0.json), and only endpoints that also exist in 12.0 are used.
- A single persisted session: the token in the Credential Manager, plus server URL and user id in the settings.
- Multi-server support, if ever needed, would require a new ADR and a session model change.
