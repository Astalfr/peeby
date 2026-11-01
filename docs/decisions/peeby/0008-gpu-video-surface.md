# 0008: Zero-copy GPU video surface

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted (2026-10-31). Supersedes [0002](0002-software-video-surface.md): the software surface is now the fallback.
- Date: 2026-10-31

## Context

The software surface (ADR 0002) works but spends about one core at 1080p60 on copying and uploading frames, and its HDR handling is limited. The pinned libmpv exposes a **D3D11 render API**, and GPUI's Windows renderer is D3D11, but GPUI has no element for external textures.

## Decision

- mpv renders with `MPV_RENDER_API_TYPE_D3D11` on its own `ID3D11Device` (adapter 0, the one GPUI picks, with multithread protection on).
- The targets are a ring of 3 BGRA textures created with `D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX`.
- A **minimal patch of `gpui-pre-windows`** ([vendor/README.md](../../../vendor/README.md), about 100 lines) lets a marker `RenderImage` stand for a shared texture:
  - GPUI opens it on its own device and draws it with the existing sprite pipeline;
  - both sides take the keyed mutex (key 0) around their GPU work.
- Video frames never go through the CPU.
- The software surface stays as a fallback behind the same `VideoSurface` trait, for example if the D3D11 path fails at startup.

## Measurements (spike, synthetic 1080p60 + ASS torture script)

Same source, same subtitles, same window size, measured back to back. CPU is the whole process, including lavfi generating the video:

| Window | Software surface | GPU surface |
|---|---|---|
| 1280x720 | 4.7–5.7 % (≈ 1.2 cores) | **0.5–0.6 % (≈ 0.13 core)** |
| 1920x1080 | 7.7–8.5 % (≈ 1.9 cores) | **0.7–1.1 % (≈ 0.22 core)** |

- That is roughly **8–9× less CPU**.
- Both paths hold 60 fps. The GPU path drops 6–7 frames at startup only (warm-up) and none afterwards.
- Resize recreates the ring. It was tested at 4 sizes with no errors.

## Consequences

- Much lower CPU use. Scaling, subtitles and color conversion run on the GPU with mpv's shaders, and the way to HDR and higher resolutions is open.
- We maintain a vendored GPUI crate. Every GPUI bump has to re-apply the patch, and `vendor/gpui-pre-windows.patch` keeps that mechanical.
- A device loss on either side needs handling in the real implementation:
  - GPUI side: the patch clears its entries;
  - mpv side: recreate the device and the render context.
- **Done (phase 3): the marker is no longer trusted from pixel data.** Handles must be registered first (`gpui_windows::register_external_texture`).
  - In the spike, any polychrome image starting with `GPUIEXT1` + u64 made GPUI open that shared handle, server artwork included.
  - Now an unregistered handle is uploaded as normal pixels.
- **Tested:** `hwdec=d3d11va` zero copy on a real H.264 file, and network playback from Jellyfin with embedded ASS.
- **Not tested yet:**
  - HEVC 10-bit;
  - device loss on either side;
  - continuous drag-resize, which needs debouncing because each step allocates 3 textures plus 3 marker buffers.
- **Keyed mutex contention, done (phase 3):** mpv renders, with its own pacing, into a private texture, which is then copied on the GPU (`CopyResource`) into the shared slot. The keyed mutex is only held for the copy submission.
- Upstreaming an "external texture" primitive to GPUI would remove the patch. That is worth proposing to Zed later.
