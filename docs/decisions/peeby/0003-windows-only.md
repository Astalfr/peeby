# 0003: Windows only

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

Supporting macOS and Linux would multiply the hardest part of the project, the video display path: Metal, Vulkan/Blade, IOSurface, dmabuf. It would also multiply packaging work.

## Decision

Target Windows 10/11 x64 only, with the MSVC toolchain and GPUI's DirectX 11 backend.

## Consequences

- One rendering backend to integrate with, one libmpv build, and Windows paths (`%APPDATA%`, `%LOCALAPPDATA%`) and the Credential Manager used directly.
- Code may use Windows-only APIs where that is simpler. Porting later would need a new ADR.
