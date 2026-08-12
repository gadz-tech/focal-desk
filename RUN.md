# Running focal-desk on Windows

## First time (about 10 minutes)

1. **Install Rust** — https://rustup.rs, take the defaults. It installs the MSVC
   toolchain; if it asks for Visual Studio Build Tools, say yes (it needs a linker).
   Open a *new* terminal afterwards so `cargo` is on PATH.

2. **Build and run**

   ```
   cd C:\Dev\focal-desk
   cargo run --release
   ```

   First build takes a few minutes (it compiles the `windows` crate). Later
   builds are seconds.

3. **Watch the console.** It prints the config path, whether desk mode is on,
   and how many windows it is managing.

## What you should see

- With the 8K panel attached, focal-desk detects it (any display 5000px or wider
  counts as the desk) and every manageable window flies to a home slot.
- Click a window — after `dwell_ms` (1.2s by default) it flies to the focal
  stage and the previous occupant returns to its own home.
- Click the desktop to clear the stage.
- `Ctrl+Alt+Space` also clears the stage.
- `Ctrl+Alt+D` forces desk mode on or off — **use this to try it on the laptop
  screen** before the panel is connected.
- Quit with `Ctrl+C` in the console. Windows stay where they are; nothing is
  permanent.

## Tuning

A `focal-desk.conf` is written next to the executable
(`target\release\focal-desk.conf`) on first run. Edit it and restart:

```
gutter_in          = 1.5    # structural gap; actively resizes windows
focal_frac         = 0.56   # width of the focal column
band_frac          = 0.22   # height of the top/bottom bands
dwell_ms           = 1200   # focus hold time before promotion
screen_diagonal_in = 65
force_active       = false  # true = manage windows without the desk display

[app]
process   = *windowsterminal*
home      = left-bottom
focal_fit = 0.55 x 1.0
```

Slots: `focal`, `left-top`, `left-bottom`, `right-top`, `right-bottom`,
`top-1`, `top-2`, `bottom-1`, `bottom-2`, `corner-tl`, `corner-tr`,
`corner-bl`, `corner-br`.

## Known rough edges (v0.1)

- **Elevated windows won't move** unless focal-desk also runs elevated. Run the
  console as administrator if you keep an admin terminal open.
- **Apps with minimum sizes** (some installers, Slack's mini player) may refuse
  their slot rect and sit oversized. They are still usable; the layout just
  isn't exact for them.
- **Windows with no resize border are left alone** — caption-less app popups and
  fixed-size dialogs keep their own size wherever they are put, so a slot would
  only ever hold them wrong. They float instead.
- **The wallpaper layer — flow field and connection wires — is not implemented
  yet.** The router that computes the wire paths is done and tested; drawing
  them behind the windows is the next piece of work. Until then, this is the
  layout engine only.
- The focal stage plus 12 home slots. A 13th window is left unmanaged (floats)
  rather than displacing anything — pinned by `engine::tests::thirteenth_window_floats`.

## If the build fails

Paste the compiler errors back into the chat. The Win32 layer now builds clean
against the `windows` crate 0.61 on stable MSVC, so a failure most likely means
a toolchain problem (missing MSVC linker) rather than the adapter itself — see
step 1 above.
