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

3. **Look in the tray, and read the log.** The service runs windowless — a
   console window has a resize border, so focal-desk would tile its own console
   into a home slot. Instead there is a notification-area icon (hover it for
   desk mode and the window count) and a log beside the executable:

   ```
   Get-Content C:\Dev\focal-desk\target\release\focal-desk.log -Wait
   ```

   Windows 11 files new tray icons under the taskbar's `^` overflow. Drag it
   onto the taskbar to keep it in sight.

## Start it at logon

```
powershell -ExecutionPolicy Bypass -File C:\Dev\focal-desk\install-task.ps1
```

Registers a scheduled task that starts focal-desk 15 seconds after you log in,
elevated so it can move admin windows too. It asks for administrator rights and
will prompt. `install-task.ps1 -Remove` undoes it.

Only one focal-desk runs at a time — a second launch notices the first and
exits, so the logon task and a manual run cannot end up fighting each other.

## What you should see

- With the 8K panel attached, focal-desk detects it (any display 5000px or wider
  counts as the desk) and every manageable window flies to a home slot.
- Click a window — after `dwell_ms` (1.2s by default) it flies to the focal
  stage and the previous occupant returns to its own home.
- Click the desktop to clear the stage.
- `Ctrl+Alt+Space` also clears the stage, as does **Clear stage** on the tray
  menu.
- `Ctrl+Alt+D` forces desk mode **on** — it does not turn it off. Use it to try
  the layout on the laptop screen before the panel is connected; with the panel
  attached, desk mode is already on and the key does nothing.
- Quit from the tray menu. Windows stay where they are; nothing is permanent.

## Tuning

A `focal-desk.conf` is written next to the executable
(`target\release\focal-desk.conf`) on first run. **Edits apply on save** — no
restart, no need to quit; the layout re-flows within about half a second, which
makes the geometry knobs something you can dial in by feel.

If an edit doesn't parse, focal-desk keeps the configuration it is already
running and writes the offending line to the log — it will not silently drop
back to defaults and lose your `[app]` rules.

Saving the file is also how you **re-assert the layout**. focal-desk does not
continuously police window positions — drag a window off its slot and it stays
there until something moves it. Re-saving the config (even unchanged) puts
every window back where it belongs, and fixes any window whose first placement
landed a few pixels out because its frame was still settling when it opened.

One caveat: `gutter_in`, `focal_frac`, `band_frac` and `dwell_ms` take effect
immediately, but a changed `home` or `focal_fit` applies to windows opened
*after* the edit. Re-homing windows already on screen is exactly the
muscle-memory breakage the layout exists to prevent.

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

## Overlays and screen capture

Screen capture is the one thing a moving layout ruins. While Snipping Tool
(or the task switcher, start menu, search) holds the foreground, focal-desk
**freezes**: nothing is promoted, nothing is placed, and any window mid-flight
stops where it is. When the overlay closes, every window is re-asserted in one
pass, so anything that drifted is put right.

The built-in ignore list covers the Windows shell surfaces. Add your own —
they're appended to the defaults:

```
[ignore]
process = *obs64*
```

## Known rough edges (v0.1)

- **Elevated windows won't move** unless focal-desk also runs elevated. The
  logon task above already does; a manual launch does not.
- **`focal-desk.conf`, the log and the executable all live in `target\release\`,
  so `cargo clean` deletes your tuning** and breaks the scheduled task's path.
  Keep a copy of the conf once you've dialled it in.
- **Maximized windows are restored before being placed.** A maximized window
  keeps drawing its frame for the screen edge it thinks it is against, so
  focal-desk restores it first. You may see a window un-maximize as it is
  adopted; that is deliberate.
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
