# CLAUDE.md — focal-desk

Conventions for anyone (human or Claude) working this repo:

- **Every function gets a `///` doc comment, no matter how simple** — a brief
  note on how it works. No undocumented functions, including test helpers.
- **focal-core stays pure.** No OS calls, no threads, no clocks. The OS talks
  to it only through `engine::Event` in and `engine::Command` out.
- **Features enter as Event/Command variants or config**, never as functions
  bolted onto platform code. See ARCHITECTURE.md ("Where a feature goes").
- **`index.html` is the playable spec.** Behavioral changes should land in both
  the mock and the engine, and engine invariants get pinned by tests.
- **Run `cargo test --workspace` before pushing.** CI runs Linux and Windows.
- Decided: clicking the desktop (shell window foreground past dwell) maps to
  `Event::ClearStage`. Clicking the focused window is a no-op (test-pinned).
- The Win32 layer (`focal-win`) is gated on `cfg(windows)`, not a Cargo feature:
  Linux CI compiles an empty crate, Windows compiles the real adapter. It has
  been built and smoke-tested on the 8K panel.
- `win::is_manageable` is the one place that decides what focal-desk owns. Keep
  it conservative — a window that cannot be resized cannot be tiled.

## Doc naming: dated = record, undated = living (added 2026-08-21 — same text lives in every C:\Dev repo's CLAUDE.md)

A date in a filename means the file is a **record of that date/window** — day plans,
handoffs, worker prompts, single-run runbooks. Records are never updated after their
window closes; new work goes in a new file. **Living documents** (plans of record,
decision records, strategies) carry **no date in the name**; they open with
`**Opened:** <date>` and accrue dated amendment sections inline — chronology lives in
the sections and in git, so there is no last-updated field to rot. When a dated file
starts accruing amendments past its window, it has changed class: rename it undated
(`git mv` + fix references) in that session. **Why (Ryan, 2026-08-21):** a dated title
on a living doc reads as stale, and two similar docs diverging is how future Ryan
loses track of which one is true when something breaks. One living doc per topic;
records frozen at their window.
