# Vendored crates

## gpui-pre-windows 0.3.10 (patched)

This is GPUI's Windows platform crate (Apache-2.0, see `gpui-pre-windows/LICENSE-APACHE`), copied from crates.io and wired in through `[patch.crates-io]` in the root `Cargo.toml`. Five files differ from upstream (`directx_atlas.rs`, `directx_renderer.rs`, `platform.rs`, `window.rs`, and `gpui_windows.rs` for the exported functions). The full diff is in [gpui-pre-windows.patch](gpui-pre-windows.patch), and every change is marked `PEEBY PATCH`.

### What the patch does

It draws **external D3D11 textures** without copying them. Peeby uses this for zero-copy video ([ADR 0008](../docs/decisions/0008-gpu-video-surface.md)).

- **`directx_atlas.rs`:**
  - A polychrome image whose bytes start with `b"GPUIEXT1"`, followed by a little-endian `u64` legacy DXGI shared handle, is not uploaded. The atlas opens the shared texture on GPUI's device (`OpenSharedResource`), creates a shader resource view, and returns a tile covering the whole texture under a reserved texture index (≥ 2³⁰).
  - Removing the tile (`Window::drop_image`) releases the texture.
  - A device loss clears external entries. They are reopened on the next paint, because the image key is gone and GPUI rebuilds it.
- **`directx_renderer.rs`:** `draw_polychrome_sprites` takes the texture's keyed mutex (key 0) around the draw when the texture has one. The raw `HRESULT` is checked, because `WAIT_TIMEOUT` is a success code. On timeout, the batch is skipped for that frame.

**Registration:** the marker is only honored for handles registered with `gpui_windows::register_external_texture(handle)` and not yet revoked with `unregister_external_texture`. An image whose bytes merely look like a marker, server artwork included, is uploaded as ordinary pixels, so data can never make GPUI open a handle. `gpui-video` registers each ring texture when it arrives and unregisters it when the ring is replaced or the view is released.

**Cursor:** `platform.rs` exports `hide_cursor_until_mouse_moves()`. GPUI already hides the cursor while typing and shows it again on the next mouse move, but only internally. The player calls this when its controls hide. It only acts when the cursor is over a window of the UI thread.

**Window bounds:** `window.rs` exports `set_window_bounds(window, cx, bounds)`, a `SetWindowPos` without activation or z-order change. GPUI only resizes from the top-left corner; the mini-player resizes from every edge (ADR 0008). The call runs as a foreground task, as GPUI's own `resize` does: `SetWindowPos` sends `WM_SIZE`/`WM_MOVE` synchronously, and from inside an update (an event, or `cx.defer`) the app is still borrowed and the new bounds would be dropped.

The polychrome sprite shader derives UVs from `GetDimensions` of the bound texture, so no shader change is needed.

### Updating GPUI

When `gpui-pre` is bumped (only together with `gpui-component`, see ADR 0006):
1. Copy the new `gpui-pre-windows-<version>` from `~/.cargo/registry/src/*/` into `vendor/gpui-pre-windows`.
2. Re-apply the patch: `git apply --directory=vendor/gpui-pre-windows vendor/gpui-pre-windows.patch`, and fix any rejects by hand.
3. Regenerate the patch file, then run `spike-gpu-video` and check the image, resizing and CPU.
