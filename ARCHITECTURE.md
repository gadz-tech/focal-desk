# Architecture

focal-desk is built so that a new feature slips into an existing seam instead of
being bolted onto the side. This document is the map of those seams.

## The shape

```text
            ┌─────────────────────────────────────────┐
 Win32      │  focal-win (adapter)                    │
 realities  │  hook · frame · anim · dock              │
            │  log · tray · adapter                    │
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
pub enum Event {
    Opened(WinId, WindowMeta), Promoted(WinId, Source), Dwelled(WinId),
    Foreground(WinId), MoveTo(WinId, SlotId), Closed(WinId),
    ClearStage, DeskMode(bool), Suspend(bool), Reconfigured(Config),
}
pub enum Source { Tab, Hotkey, Drop, Dwell }
pub enum Command { Place { win: WinId, to: Rect, animate: bool }, Release(WinId) }
```

Two of those are deliberately inert. `Foreground` is what the OS foreground hook
becomes: the engine records it and never places anything for it — a click into
a side window, a copy there and a paste elsewhere, move nothing (2026-09-04,
REQUESTS §1/§2). `Dwelled` is the hold-to-promote timer, and the engine vetoes
it while `dwell_ms = 0`. Every `Promoted` names its `Source`, and the adapter
logs it, so a window that reaches the stage uninvited says who sent it.

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
| add a promotion gesture (hotkey, gaze, …)      | adapter only — it sends `Promoted(win, Source::…)` through `Service::promote`, adding a `Source` variant if none fits; the log names it |
| add wire behaviors (pulse on notification)     | new `Command` variant + renderer          |
| leave an app alone entirely                    | `[ignore]` in config — no code            |
| freeze the layout for a new kind of overlay    | adapter raises `Event::Suspend`           |
| swap the whole layout (strip, twin-focal, …)   | `layout.rs::regions` — same slot ids      |
| draw/remove soft wires                         | new `Event` variants, engine state        |
| move a window by hand (2026-09-03: drag its tab) | adapter sends `Event::MoveTo(win, slot)`; `layout::slot_at` names the slot under the pointer |
| change what the frame looks like (2026-09-04)  | `tab.rs` in core lays it out (`Frame`: slim ×3 + thick toward center, strip, holes, the §7 box's place), `paint.rs` in core rasterizes a band of it — both pure, tested on Linux; `focal-win/tab.rs` only hosts four band windows per frame and blits the pixels |
| give an app a slot preference or priority (§8) | `[app]` in config: `slots = corners` / `top, bottom`, `priority = true`; the engine's `on_opened` and `displace` — a hand drag pins a window against every rule |

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

## Two invariants learned the hard way

**Measure frame insets once.** `frame::measure` is only ever called on a
restored, stationary window, and the adapter caches the result for the life of
that window. Re-measuring one that is mid-flight, maximized, or freshly torn
out of a browser tab reads bounds DWM has not settled; feeding that into the
next placement compounds a few pixels per promotion until the frame is visibly
wrong. Implausible measurements are discarded rather than trusted.

**Freeze for overlays.** Anything matching the ignore list is never managed,
and while one holds the foreground the engine issues no commands at all and
in-flight animations are settled immediately. A screen capture must see a still
screen.

## The Win32 layer (`focal-win`)

Compiled and smoke-tested on Windows. Its modules are gated on `cfg(windows)`
(not a Cargo feature), so the crate is an empty shell on Linux CI and the real
adapter everywhere else. The four things worth knowing:

- **`hook.rs`** — one `SetWinEventHook` over `EVENT_SYSTEM_FOREGROUND ..=
  EVENT_SYSTEM_MINIMIZEEND` plus the tabs in `tab.rs` are the entire input
  side. Since 2026-09-04 a foreground change is informational (it drives the
  frames' z-order re-assert and, only while `dwell_ms > 0`, the dwell timer);
  the tab tap, the tab drag and the hotkeys are the gestures.
- **`tab.rs`** — one layered, non-activating window per managed window, sitting
  *directly below* it in z-order (never topmost, 2026-09-04 §4): the window
  covers the inner region, the ring outside is what you tap or drag. Hidden
  while frozen, under a fullscreen foreground, and when its own window is
  minimized, hidden or off the managed monitor. The drag is polled from the
  main loop, not captured.
- **`frame.rs`** — `GetWindowRect` lies (~7px invisible resize borders, varies
  per app). Measure `DWMWA_EXTENDED_FRAME_BOUNDS` and compensate, or every
  gutter looks ragged.
- **`anim.rs` + `snapshot.rs`** — flights run our own ease-out loop;
  `SWP_NOACTIVATE | SWP_NOZORDER` so a flight never steals focus (promotion
  must not feed itself) or reorders anything. Since 2026-09-04 (§9) what flies
  is a DWM thumbnail of the window in a host window of ours; the real window
  is resized **once**, when the flight lands, and the likeness lingers two
  frames so the app can paint at its new size. Fusion and KiCad re-create
  their GPU surfaces on every resize and broke under the old per-frame one.
- **`dock.rs`** — desk mode = the 7680x4320 panel is present. Re-checked on
  `WM_DISPLAYCHANGE`/`WM_DEVICECHANGE`; transitions become `Event::DeskMode`,
  and undocking releases every window (`Command::Release`) — the service is
  inert on the laptop.
- **`tray.rs`** — the only UI. The binary is windowless (a console has a resize
  border, so the service would tile its own console), so the icon is what says
  it is running, and `log.rs` is what says what it is doing. Both hang off the
  same hidden window as the hotkeys.

Config edits reload live: the adapter watches the file's mtime on the same
400 ms tick as the window rescan and sends `Event::Reconfigured`. That is the
seam a settings GUI would use — write the file, and the layout follows.

Also planned in this layer: the wallpaper renderer (flow field + wires drawn on
the WorkerW layer behind windows), fed by the same engine state.

## CI

GitHub Actions runs `cargo test --workspace` on Linux and Windows for every
push. The Windows job is what really compiles the adapter, since everything in
`focal-win` sits behind `cfg(windows)`.
