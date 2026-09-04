# CLAUDE.md — focal-desk

Conventions for anyone (human or Claude) working this repo:

- **Every function gets a `///` doc comment, no matter how simple** — a brief
  note on how it works. No undocumented functions, including test helpers.
- **focal-core stays pure.** No OS calls, no threads, no clocks. The OS talks
  to it only through `engine::Event` in and `engine::Command` out.
- **Features enter as Event/Command variants or config**, never as functions
  bolted onto platform code. See ARCHITECTURE.md ("Where a feature goes").
- **`index.html` is retired (Ryan, 2026-09-03).** Two generations behind the
  engine and nothing real could be tested with it — it only ever showed windows
  moving. Don't touch it, don't port behavior into it. **The focal-core tests
  are the spec:** a behavioral change lands in the engine with the test that
  pins it. (Mock-first was by design in 2026-08; today's word wins.)
- **Run `cargo test --workspace` before pushing.** CI runs Linux and Windows.
- Decided: clicking the desktop (shell window foreground past dwell) maps to
  `Event::ClearStage`. Clicking the focused window is a no-op (test-pinned).
- The Win32 layer (`focal-win`) is gated on `cfg(windows)`, not a Cargo feature:
  Linux CI compiles an empty crate, Windows compiles the real adapter. It has
  been built and smoke-tested on the 8K panel.
- `win::is_manageable` is the one place that decides what focal-desk owns. Keep
  it conservative — a window that cannot be resized cannot be tiled.

<!-- SHARED-RULES:EXEMPT — this repo is PUBLIC (github.com/gadz-tech/focal-desk); the private
     shared-rules block is deliberately not vendored here. Marked 2026-09-04 by Worker F on
     merging feat/frames: the block had been stamped in on 2026-08-21 and re-synced 2026-09-01. -->
