# 0007: Hand-written libmpv FFI, loaded at runtime

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

- `libmpv2` 7.0, the maintained Rust binding, only wraps the OpenGL render API. We need the SW API now, and maybe D3D11 later.
- The shinchiro package ships a MinGW import library, not an MSVC `.lib`.
- bindgen would add a libclang build dependency for about 20 functions.

## Decision

`player::sys` declares the few libmpv functions, structs and constants we use by hand, checked against the pinned headers in `third_party/mpv/include/`. They are resolved at runtime from `libmpv-2.dll` with `libloading`.

## Consequences

- No import library and no build-time link step. A missing or incompatible DLL becomes a clean startup error instead of a loader crash.
- Constants and struct layouts must be re-checked when libmpv is bumped, using `MPV_CLIENT_API_VERSION` in client.h, currently 2.6. The `player` crate checks `mpv_client_api_version()` at load.
- Every FFI call sits in one module with `// SAFETY:` comments, as CLAUDE.md requires.
