# 0004: GPL-3.0-or-later

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

The ready-made Windows libmpv builds are GPL, with FFmpeg built with GPL components. Shipping them makes the distributed application a GPL combination. An LGPL build would mean building mpv and FFmpeg ourselves.

## Decision

License Peeby under GPL-3.0-or-later and use the standard GPL libmpv builds.

## Consequences

- No custom mpv/FFmpeg builds to maintain.
- Distribution has to include the license texts and a source offer for libmpv/FFmpeg (licenses.md (peeby's `docs/licenses.md`)).
- Every dependency must be GPL-3.0-compatible. Apache-2.0, MIT, BSD, ISC and MPL-2.0 are fine.
