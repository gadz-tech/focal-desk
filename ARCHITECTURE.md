# Architecture

focal-desk is built so that a new feature slips into an existing seam instead of
being bolted onto the side. This document is the map of those seams.

## The shape

```text
            ┌─────────────────────────────────────────┐
 Win32      │  focal-win (adapter)                    │
 realities  │  hook · frame · anim · dock · adapter   │
            └───────────────┬───────────▲─────────────┘
                     Event  │           │  Command
                            ▼           │
            ┌─────────────────────────────────────────┐
            │  focal-core (the brain — no OS, no      │
            │  threads, no clocks)                    │
            │  layout · engine · wires · config       │
            └─────────────────────────────────────────┘
```

`focal-core` is a pure state machine. The adapter feeds it `Event`s and executes
the `Command`s that come back. That contract is the whole architecture:

```rust
pub enum Event   { Opened(WinId, WindowMeta), Promoted(WinId), Closed(WinId), DeskMode(bool) }
pub enum Command { Place { win: WinId, to: Rect, animate: bool }, Release(WinId) }
```

Three properties fall out of this:

1. **Testable anywhere.** The entire behavior — promotion, muscle-memory homes,
   focal fit, the 13th-window policy, undock — is unit-tested on Linux CI with
   no Windows in sight. `cargo run` prints a narrated simulation of the engine.
2. **Extensible by construction.** A feature is a new `Event` variant, a new
   `Command` variant, or logic between the two. The compiler then points at
   every place that must care (exhaustive `match`), which is the opposite of a
   function bolted on somewhere findable only by grep.
3. **Swappable edges.** The mock in `index.html`, the demo binary, and the real
   Win32 adapter are all just different drivers of the same brain.

## Where a feature goes

| You want to…                                   | Touch                                     |
| ---------------------------------------------- | ----------------------------------------- |
| change dwell time, gutter, tier sizes          | `config.rs` (data, no code)               |
| give an app a home or a focal fit              | `config.rs` — add an `AppRule`            |
| change what happens to a 13th window           | one arm in `engine.rs::on_opened`         |
| add a promotion gesture (hotkey, gaze, …)      | adapter only — it just sends `Promoted`   |
| add wire behaviors (pulse on notification)     | new `Command` variant + renderer          |
| swap the whole layout (strip, twin-focal, …)   | `layout.rs::regions` — same slot ids      |
| draw/remove soft wires                         | new `Event` variants, engine state        |

## Layout: the structural gutter

Thirteen regions tile the screen exactly (`layout.rs` has the diagram); every
window is its region inset by half a gutter. Adjacent regions share an edge, so
windows are always exactly one gutter apart, gutters form straight continuous
channels, and the wire router can treat them as a known corridor network. The
gutter is configured in **inches** (1.5" ≈ 203px at 8K/65") because it's a
physical, perceptual quantity — it should survive a resolution change.

Tests in `layout.rs` pin these invariants; treat them as the spec.

## The wire router (`wires.rs`)

Priority order: never under a window → prefer a private lane (crossings OK,
parallel sharing not — PCB rule) → share a lane when squeezed → report
unroutable (renderer draws stubs). Wires leaving the same window edge fan out
into distinct ports ordered by destination. Routing only recomputes when the
layout changes, which is only on promotion — idle cost is zero.

## The Win32 layer (`focal-win`)

Reference code today, gated behind the `win32` feature; enable on a Windows
machine (dep instructions in its Cargo.toml). The four things worth knowing:

- **`hook.rs`** — one `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` is the entire
  input side. Activation is the gesture; there is no other UI.
- **`frame.rs`** — `GetWindowRect` lies (~7px invisible resize borders, varies
  per app). Measure `DWMWA_EXTENDED_FRAME_BOUNDS` and compensate, or every
  gutter looks ragged.
- **`anim.rs`** — flights run our own ease-out loop; `SWP_NOACTIVATE` so a
  flight never steals focus (promotion must not feed itself), batched through
  `DeferWindowPos`.
- **`dock.rs`** — desk mode = the 7680x4320 panel is present. Re-checked on
  `WM_DISPLAYCHANGE`/`WM_DEVICECHANGE`; transitions become `Event::DeskMode`,
  and undocking releases every window (`Command::Release`) — the service is
  inert on the laptop.

Also planned in this layer: the wallpaper renderer (flow field + wires drawn on
the WorkerW layer behind windows), fed by the same engine state.

## CI

GitHub Actions runs `cargo test --workspace` on Linux and Windows for every
push. The Windows job is what really compiles the adapter once `win32` is on.
