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
- The Win32 layer (`focal-win`) is reference code behind the `win32` feature
  until first built on a real Windows machine.
