# 0029: The shell and each space in a crate of their own, for build times

- Status: accepted
- Date: 2026-11-07

## Context

The debug build was slow to rebuild: measured on 2026-11-07, `cargo build -p peeby` after a
one-line change took 17–36 s, and 7–9 s with nothing to compile but the link. The `peeby`
binary held 36,000 of the workspace's 69,000 lines: the shell, the settings, the desktop
integration, the updates, the sign-in window and the three spaces. rustc's profile of that
crate (`-Z time-passes`, from scratch) was about 16 s of front end (macro expansion 4.7 s,
type checking 2.7 s, borrow checking 2.6 s, the monomorphization collector 3.7 s), 22 s of
code generation and 21 s of LLVM, partly in parallel, and 4 s of linking. Incremental
compilation saved little: macro expansion and most of the front end run again on every
change, and GPUI's builder generics are instantiated in the crate itself, since the
dependencies are built at `opt-level = 2`, which does not share generics.

Options considered:

- less debug info alone (`debug = "line-tables-only"` for the dev profile, done as well): a
  third off every rebuild (21 → 14 s for the same change), the PDB down from 537 MB;
- another linker (`rust-lld`): no gain measured;
- splitting the binary along its existing boundaries. The shell already knows no service
  (spaces plug into `shell::space::Space`), and no space uses another. The shell, `settings`,
  `desktop` and `update` refer to each other, so they stay together.

## Decision

The binary is split into:

- `peeby-core`: `shell`, `settings`, `desktop`, `update`, `web_sign_in`, `logs`; it knows no
  service;
- `space-youtube`, `space-twitch`, `space-jellyfin`: one crate per space, each depending on
  `peeby-core` (never on another space);
- `peeby`: `main.rs` only (and the build script and the Twitch capture example), which puts
  them together.

Module paths inside each crate stay what they were (`crate::shell::…` in `peeby-core`); a space
reaches the shell as `peeby_core::shell::…`. A space's submodules are private unless `main.rs`
needs them (`convert`, `prefetch`, `tools`, `session`), so that `missing_docs` only asks for
what crosses a crate boundary. Each space's `Nav` is a newtype over `peeby_core::shell::space::Nav`
(with `Deref`), since its methods (`open`, `open_in_new_tab`, `set_title`) are the space's and
an inherent `impl` cannot be written outside the type's crate. The log filter names every
crate (`peeby_core`, `space_*`): a target is its crate's name.

## Consequences

- A change in a space rebuilds that space and the binary: 8–12 s measured, instead of
  17–36 s. The spaces build in parallel.
- A change in `peeby-core` rebuilds every space after it (cargo rebuilds a crate's dependents
  whenever it changes): 18–32 s, no better than before. Splitting the shell further would help
  there.
- What a space uses from the shell is now `pub` and documented; something a space needs from
  the core cannot stay `pub(crate)`.
- Adding a service means a new `space-<name>` crate depending on `peeby-core`, added to
  `peeby`'s dependencies and to the list in `main.rs`.
