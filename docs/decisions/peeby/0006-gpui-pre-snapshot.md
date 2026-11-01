# 0006: GPUI from the `gpui-pre` snapshot

> Archived from peeby (`D:\DEV\peeby`, abandoned, ADR 0007). Numbers and links are peeby's.

- Status: accepted
- Date: 2026-10-31

## Context

GPUI is published in several forms:
- `gpui` 0.2.2 on crates.io, which is old;
- `gpui-pre`, crates.io snapshots of the Zed repository;
- community forks (`gpui-ce`, `gpui-fast`).

`gpui-component` 0.8.1, the widget library we want, depends on `gpui-pre =0.3.10` (zed@08cbb88).

## Decision

Depend on GPUI as follows:

```toml
gpui = { package = "gpui-pre", version = "=0.3.10" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.10" }
```

The version is pinned exactly and moves only together with `gpui-component`.

## Consequences

- Code uses the usual `gpui::` paths, and `gpui-component` can be added without version conflicts.
- Upgrades are deliberate: bump both crates, re-run the spike, and record the change here or in a new ADR.
- `surface()` is macOS-only. Video goes through `paint_image` (see phase 0 report (peeby's `docs/spikes/phase-0.md`)).
