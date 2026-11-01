# 0002: Software video surface for the MVP

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: superseded by [0008](0008-gpu-video-surface.md). The software surface is kept as the fallback.
- Date: 2026-10-31

## Context

GPUI has no stable element for an external GPU texture. 4K, HDR and Dolby Vision are **not required**. The options are compared in [video-surface.md](../../player/video-surface.md).

## Decision

Render with the libmpv software render API into a BGRA buffer sized to the element, and show it as a GPUI image (option A), behind a `VideoSurface` trait.

## Consequences

- No GPUI fork, and UI overlays on the video are plain GPUI elements.
- There is a CPU cost per frame. HDR is shown with basic tone mapping only, which is acceptable since HDR is out of scope.
- If spike 0a fails its criterion (no drops at 1080p 24/30/60, CPU under about 20–25 %), this ADR is superseded by one choosing option B or C.
