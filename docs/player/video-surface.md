# Video surface: getting frames into GPUI

> From peeby's docs (2026-11-01), being adapted to Peeby's Jellyfin space and shared player (docs/refactor-plan.md, steps 3 and 6). Numbered ADR links point to peeby's decisions, archived in [decisions/peeby/](../decisions/peeby/).

GPUI (`gpui-pre 0.3.10`, [ADR 0006](../decisions/peeby/0006-gpui-pre-snapshot.md)) has **no element for an external GPU texture on Windows**: `surface()` is macOS-only. Its Windows renderer is DirectX 11. Verified in phase 0, see the report (peeby's `docs/spikes/phase-0.md`):
- A changing image is drawn with `canvas()` + `Window::paint_image`. Each frame is a new `RenderImage`, so it is a new sprite-atlas upload. The previous one must be released with `Window::drop_image`.
- The pinned libmpv exposes the render APIs `sw`, `opengl` **and `d3d11`**. The D3D11 one makes option B a zero-copy path without any GL bridge.

## Options

| # | Approach | Pros | Cons |
|---|---|---|---|
| **A** | **libmpv SW render API**: mpv writes BGRA into our buffer, which is shown as a GPUI image each frame | Simple, fully inside GPUI, UI overlays are trivial | One CPU→GPU copy per frame. Scaling and color conversion run on the CPU. Fine for 1080p, unproven for 4K |
| B | **GPU interop**: patch GPUI with a "shared D3D11 texture" element, and mpv renders through its D3D11 render API | Zero copy, best quality | Maintaining a GPUI patch. `render_d3d11.h` is missing from the shinchiro dev package and must come from mpv's source |
| C | **Native child window**: mpv draws into its own HWND (`wid`) | mpv quality and speed for free | "Airspace": GPUI cannot draw over the video, so controls need a second transparent window. Fragile with focus, resize and fullscreen |
| D | **Custom pipeline**: FFmpeg + libass + cpal | Full control | Re-implements A/V sync, seeking, buffering and audio that mpv already does |

## Option B prototype (works)

`spikes/gpu-video` implements B: mpv's D3D11 API renders into a ring of 3 keyed-mutex shared textures, and the patched `gpui-pre-windows` ([vendor/README.md](../../vendor/README.md)) draws them through a marker `RenderImage`. At the same window size and content, CPU drops 8–9×: from about 1.9 to about 0.22 core at 1080p with the ASS torture script. Resizing works. [ADR 0008](../decisions/peeby/0008-gpu-video-surface.md) proposes making it the main path, with A as the fallback.

## Decision

**B is the main path, A the fallback** ([ADR 0008](../decisions/peeby/0008-gpu-video-surface.md), which supersedes [ADR 0002](../decisions/peeby/0002-software-video-surface.md)). Both sit behind a `VideoSurface` trait, so the rest of the app does not know which one is in use.

- `D3d11Surface` is tried first.
- If it fails, the player falls back to `SwSurface` and logs the reason. Failures include: creating the D3D11 device, creating the mpv D3D11 render context, opening the shared texture on GPUI's side, or an unrecoverable device loss.
- The fallback is also available on demand (a setting or an environment variable) for diagnosing GPU issues.

### `D3d11Surface`

- mpv renders with `MPV_RENDER_API_TYPE_D3D11` on its own device (adapter 0, multithread protected), using `hwdec=auto` (d3d11va, zero copy).
- The targets are a ring of 3 BGRA textures with `SHARED_KEYEDMUTEX`, at the element's physical size.
- Each texture is **registered explicitly** with the patched `gpui-pre-windows`. Image bytes are never trusted as a marker ([ADR 0008](../decisions/peeby/0008-gpu-video-surface.md)).
- Render with `BLOCK_FOR_TARGET_TIME=0` and pace from the update callback, so the keyed mutex is only held for the GPU submission.
- Resizes are debounced before recreating the ring.
- Device loss: GPUI side, the patched atlas clears its entries and they are re-registered. mpv side, recreate the device, the render context and the ring.

### `SwSurface` (fallback)

- Uses `hwdec=auto-copy`.

- The buffer is sized to the element's **physical** pixel size (DPI-aware), not to the video size. This keeps ASS text sharp, see [subtitles-ass.md](subtitles-ass.md).
- Double buffering: mpv renders into the back buffer off the UI thread, and the UI shows the front buffer, so the UI thread never waits on mpv.
- `mpv_render_context_set_update_callback` signals a new frame. The UI then calls `window.request_animation_frame()` / `cx.notify()`.
- A resize reallocates the buffers. mpv re-renders at the new size, and the subtitles are re-laid out.

### Measured cost (phase 0a)

At 1080p60:
- mpv conversion and scaling take about **1 ms per frame**;
- the alpha fix takes about 0.4 ms;
- the whole process uses about one core (≈ 4–5 % of a 24-thread machine), with no drops;
- an extreme ASS script raises render cost to 12–14 ms per frame. It still holds 60 fps, but this is where the software path would saturate first.

### Option C

The native child window stays unused. It would only come back if the GPUI patch became impossible to maintain, and that would need a new ADR.
