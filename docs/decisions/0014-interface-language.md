# 0014: Interface language, English and French at each call site

- Status: accepted
- Date: 2026-11-02

## Context

Peeby's interface was written in French only (CLAUDE.md: "UI strings in French"). The user asked
for a translation system and a language setting, with English as the default, and the whole app
translated.

The UI has about a thousand strings, spread over the `ui` crate and every space. Many of them are
formatted (counts, sizes, durations, error causes) and some depend on plural rules or on the
typography of the language (French puts a space before `:` and uses « »). Many are built where
no GPUI context is at hand (`format_size`, `format_step`, a section builder taking `&Settings`).

Options considered:

- **A crate** (`fluent`, `rust-i18n`, which gpui-component uses itself) with keyed catalogues.
  It means a new dependency, a thousand invented keys kept in step with two files, and missing
  keys found only at runtime.
- **Keyed tables in our own code**: the same drift between keys and texts, without a dependency.
- **Both texts at the call site**: `t("Settings", "Réglages")` and
  `tf!("{n} videos", "{n} vidéos")` (a `format!` per language). The compiler checks the format
  arguments of each language, plurals and typography are written out where they are used, and
  the French text that was there stays next to its English.

## Decision

- `ui::i18n` holds the interface's language (`Lang::En`, `Lang::Fr`) in a process-wide atomic,
  readable without a context, and offers `t`, `tf!` and `Lang::pick`. English is the default.
- The setting is `interface_language` in `settings.toml` (`en`, `fr`; unknown codes mean
  English), chosen in the gear's Général section. On the first start (no `settings.toml` yet)
  it follows Windows' display language (`GetUserDefaultUILanguage`): French for any French,
  English for any other; it is saved with the settings' next write. It is applied before the UI starts; a change
  sets it and calls `App::refresh_windows`. Texts a page keeps in its state (a status, a loaded
  error) follow on its next load.
- It is not YouTube's content language (`language`/`region`, the `hl`/`gl` of InnerTube),
  which stays a YouTube setting, now labelled as the content language. What services send
  (titles, YouTube's "il y a 3 jours", Jellyfin's genres) keeps the language they send it in.
- Logs stay in English. Tests never change the language (they run in parallel) and expect the
  English texts.
- gpui-component's own strings (`gpui_component::set_locale`) have no French catalogue in 0.8.1
  and stay English.

## Consequences

- Adding or changing a string means writing it in both languages at once, in the same place.
- A third language means touching every call site (a third argument), or moving to catalogues
  then: the call sites already list every string with its English text, which would become the
  keys.
- No new dependency.
