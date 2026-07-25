# focal-desk

> **This is a concept mock** — a playable, self-contained HTML simulation of the layout system. It does not manage real windows (yet). The real implementation — a Rust service driving actual Windows apps — is the next step.

![Focal-desk mock: a simulated 65-inch desktop with a large focal window, satellite home slots, and glowing connection wires routed through the gutters](screenshot.png)

An interactive concept mock for a **focal-priority window layout** on very large screens, built around a 65″ 8K panel.

The idea: on a screen that big, your eyes live in the center — the corners are peripheral vision. So instead of edge-snapping and maximized zones, every app owns a permanent **home slot** arranged in a bullseye around a large **focal slot**. Activating a window promotes it to the focal slot; the previous occupant flies back to its own home, so spatial muscle memory never breaks. The focal slot is a stage, not a size: apps carry a **focal-fit** hint, so a terminal promotes to a tall column rather than a full-width sprawl. Windows are spaced by a structural **gutter** (set in real inches) that actively resizes windows and belongs exclusively to the **connection wires** — glowing, PCB-style routed links between related apps (hard links auto-detected, soft links drawn by hand; multiple wires leaving one window get their own ports along the edge). A slow flow-field drifts through the gaps, a nod to the Swordfish monitor rig. Panel model inspired by Meta Orion, which I personally worked on.

**Try it:** [play it in your browser](https://gadz-tech.github.io/focal-desk/) — or open `index.html` locally. Press F11 either way; single file, no dependencies.

Controls: click a window (or `Tab` / number keys `1–9 0 - =`) to promote it; `Esc` clears focus; `C` toggles connect mode — click two windows to link or unlink them; `F` fullscreen; `?` shows all keys. Sliders set focal size, band height, and gutter width.

The service is now taking shape in [`crates/`](crates/) — `focal-core` is the OS-free brain (layout, promotion, wire routing; fully unit-tested, `cargo run` prints a narrated demo) and `focal-win` is the Win32 adapter. Design map: [ARCHITECTURE.md](ARCHITECTURE.md).

Planned real-world behavior: a small Rust service using a foreground-event hook for promotion, a slot config, animated `SetWindowPos` transitions, and a wallpaper-layer renderer for the flow and wires. The service is dock-aware: inactive on the laptop's own display, active by default in desk mode when the eGPU (GeForce RTX 5060 Ti) and the 65″ panel are attached — detected via display-topology and device hot-plug events, releasing all windows to normal behavior on undock.
