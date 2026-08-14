//! The promotion state machine. One event in, zero or more commands
//! out; no OS types anywhere. This is the file to read to understand
//! what focal-desk *does*.

use std::collections::HashMap;

use crate::config::{Config, Fit, WindowMeta};
use crate::geometry::Rect;
use crate::layout::{self, SlotId};

/// Opaque window identity. On Windows this carries the HWND value.
pub type WinId = u64;

/// Everything the OS layer can tell the engine.
#[derive(Clone, Debug)]
pub enum Event {
    /// A manageable top-level window appeared.
    Opened(WinId, WindowMeta),
    /// A window held the foreground for the dwell period. (The adapter
    /// owns the timer; the engine only ever sees the decision point.)
    Promoted(WinId),
    Closed(WinId),
    /// The user asked for an empty stage (hotkey, or the desktop itself
    /// became foreground): send the focused window home, focus nothing.
    ClearStage,
    /// Desk mode toggled: eGPU / 65" panel attached or removed.
    DeskMode(bool),
    /// Freeze or unfreeze the layout. Raised while an ignored overlay
    /// holds the foreground — a screen capture, the task switcher —
    /// where any window motion would land in whatever the user is
    /// capturing or choosing from.
    Suspend(bool),
    /// The config file changed on disk and reparsed cleanly. Carries the
    /// whole new config; the adapter has already resolved `screen` from
    /// the live display, so the engine adopts this verbatim.
    Reconfigured(Config),
}

/// Everything the engine can ask the OS layer to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Place { win: WinId, to: Rect, animate: bool },
    /// Stop managing a window: restore normal user control.
    Release(WinId),
}

pub struct Engine {
    cfg: Config,
    active: bool,
    homes: HashMap<WinId, SlotId>,
    fits: HashMap<WinId, Option<Fit>>,
    occupants: HashMap<SlotId, WinId>,
    focused: Option<WinId>,
    suspended: bool,
}

impl Engine {
    /// A fresh engine: inactive (laptop mode), managing nothing.
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            active: false,
            homes: HashMap::new(),
            fits: HashMap::new(),
            occupants: HashMap::new(),
            focused: None,
            suspended: false,
        }
    }

    /// The single entry point.
    pub fn handle(&mut self, ev: Event) -> Vec<Command> {
        match ev {
            Event::DeskMode(on) => self.set_active(on),
            Event::Opened(id, meta) => self.on_opened(id, meta),
            Event::Promoted(id) => self.on_promoted(id),
            Event::Closed(id) => self.on_closed(id),
            Event::ClearStage => self.on_clear_stage(),
            Event::Suspend(on) => self.set_suspended(on),
            Event::Reconfigured(cfg) => self.on_reconfigured(cfg),
        }
    }

    /// The window currently on the focal stage, if any.
    pub fn focused(&self) -> Option<WinId> {
        self.focused
    }
    /// The home slot assigned to a managed window.
    pub fn home_of(&self, id: WinId) -> Option<SlotId> {
        self.homes.get(&id).copied()
    }
    /// True while the layout is frozen for an overlay.
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Freeze or resume. Resuming re-asserts every window's position in
    /// one pass, so anything that drifted while frozen — or opened
    /// during a capture — is put right the moment the overlay closes.
    fn set_suspended(&mut self, on: bool) -> Vec<Command> {
        if on == self.suspended {
            return Vec::new();
        }
        self.suspended = on;
        if on || !self.active {
            return Vec::new();
        }
        self.reassert()
    }

    /// A `Place` for every managed window at the position it should
    /// currently hold.
    fn reassert(&self) -> Vec<Command> {
        let focused = self.focused;
        let cfg = &self.cfg;
        self.homes
            .iter()
            .map(|(&id, &slot)| {
                let to = if Some(id) == focused {
                    layout::focal_rect(cfg, self.fits.get(&id).copied().flatten())
                } else {
                    layout::window_rect(cfg, slot)
                };
                Command::Place { win: id, to, animate: false }
            })
            .collect()
    }

    /// True in desk mode, i.e. the engine is issuing commands.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Read-only view of the active configuration.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// Adopt a new screen (resolution learned at runtime, or changed by
    /// docking) and re-place every managed window to match.
    pub fn set_screen(&mut self, screen: crate::config::Screen) -> Vec<Command> {
        self.cfg.screen = screen;
        self.replace_all()
    }

    /// Re-place every managed window under the current config: the
    /// focused one on the focal stage at its fit, everyone else at its
    /// own home. Snaps rather than animates — this runs on resolution
    /// changes and live config retunes, where a flight would only lag
    /// behind the change. An inactive engine commands nothing.
    fn replace_all(&self) -> Vec<Command> {
        if !self.active {
            return Vec::new();
        }
        let focused = self.focused;
        let cfg = &self.cfg;
        self.homes
            .iter()
            .map(|(&id, &slot)| {
                let to = if Some(id) == focused {
                    layout::focal_rect(cfg, self.fits.get(&id).copied().flatten())
                } else {
                    layout::window_rect(cfg, slot)
                };
                Command::Place { win: id, to, animate: false }
            })
            .collect()
    }

    /// Adopt a new configuration and re-place everyone under it.
    ///
    /// Home assignments and focus deliberately survive: retuning the
    /// gutter must not shuffle which app lives where, which is the whole
    /// muscle-memory promise. A changed `[app]` rule therefore applies to
    /// windows opened after the edit, not to ones already placed.
    fn on_reconfigured(&mut self, cfg: Config) -> Vec<Command> {
        self.cfg = cfg;
        self.replace_all()
    }

    /// Desk-mode transition. Entering: every managed window flies to
    /// its home, focus cleared. Leaving: every window is released back
    /// to normal user control.
    fn set_active(&mut self, on: bool) -> Vec<Command> {
        self.active = on;
        let cfg = &self.cfg;
        if on {
            // Entering desk mode: everyone flies to their home.
            self.focused = None;
            self.homes
                .iter()
                .map(|(&id, &slot)| Command::Place {
                    win: id,
                    to: layout::window_rect(cfg, slot),
                    animate: true,
                })
                .collect()
        } else {
            // Undocking: hands off every window.
            self.focused = None;
            self.homes.keys().map(|&id| Command::Release(id)).collect()
        }
    }

    /// Adopt a new window: home = the first matching rule's preferred
    /// slot if free, else the first free slot center-out. With no free
    /// slot the window stays unmanaged (floats).
    fn on_opened(&mut self, id: WinId, meta: WindowMeta) -> Vec<Command> {
        // Capture overlays and shell surfaces are left entirely alone.
        if self.cfg.is_ignored(&meta) {
            return vec![Command::Release(id)];
        }
        let rule = self.cfg.apps.iter().find(|r| r.matcher.matches(&meta));
        let fit = rule.and_then(|r| r.focal_fit);
        let preferred = rule.and_then(|r| r.home);
        // Preferred slot if free, else first free slot center-out.
        let slot = preferred
            .filter(|s| !self.occupants.contains_key(s))
            .or_else(|| {
                layout::home_priority()
                    .into_iter()
                    .find(|s| !self.occupants.contains_key(s))
            });
        let Some(slot) = slot else {
            // Thirteenth window: no home exists. Current policy is to
            // leave it floating. (To change the policy, change only
            // this arm — nothing else in the system cares.)
            return vec![Command::Release(id)];
        };
        self.homes.insert(id, slot);
        self.fits.insert(id, fit);
        self.occupants.insert(slot, id);
        if self.active && !self.suspended {
            vec![Command::Place {
                win: id,
                to: layout::window_rect(&self.cfg, slot),
                animate: true,
            }]
        } else {
            Vec::new()
        }
    }

    /// Put a window on the focal stage (shrunk to its fit hint) and
    /// send the previously focused window back to its own home.
    fn on_promoted(&mut self, id: WinId) -> Vec<Command> {
        if !self.active
            || self.suspended
            || !self.homes.contains_key(&id)
            || self.focused == Some(id)
        {
            return Vec::new();
        }
        let mut cmds = Vec::new();
        if let Some(prev) = self.focused {
            if let Some(&slot) = self.homes.get(&prev) {
                // The displaced window returns to ITS OWN home — never
                // to someone else's slot. This is the muscle-memory
                // invariant: Mail always lives where Mail lives.
                cmds.push(Command::Place {
                    win: prev,
                    to: layout::window_rect(&self.cfg, slot),
                    animate: true,
                });
            }
        }
        let fit = self.fits.get(&id).copied().flatten();
        cmds.push(Command::Place {
            win: id,
            to: layout::focal_rect(&self.cfg, fit),
            animate: true,
        });
        self.focused = Some(id);
        cmds
    }

    /// Empty the stage: the focused window flies home; nothing focused.
    /// Triggered by a hotkey or by the desktop itself becoming
    /// foreground (decided: clicking the desk clears the stage).
    fn on_clear_stage(&mut self) -> Vec<Command> {
        if self.suspended {
            return Vec::new();
        }
        let Some(id) = self.focused.take() else {
            return Vec::new();
        };
        match self.homes.get(&id) {
            Some(&slot) => vec![Command::Place {
                win: id,
                to: layout::window_rect(&self.cfg, slot),
                animate: true,
            }],
            None => Vec::new(),
        }
    }

    /// Forget a closed window and free its slot for reuse.
    fn on_closed(&mut self, id: WinId) -> Vec<Command> {
        if let Some(slot) = self.homes.remove(&id) {
            self.occupants.remove(&slot);
        }
        self.fits.remove(&id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppRule, Matcher};

    /// Shorthand: a WindowMeta with just a process name.
    fn meta(p: &str) -> WindowMeta {
        WindowMeta { process: p.into(), title: String::new() }
    }

    /// An engine whose config gives terminals a 0.55-wide focal fit.
    fn engine_with_terminal_rule() -> (Engine, Config) {
        let mut cfg = Config::default();
        cfg.apps = vec![AppRule {
            matcher: Matcher::Process("*terminal*".into()),
            home: None,
            focal_fit: Some(Fit { w: 0.55, h: 1.0 }),
        }];
        (Engine::new(cfg.clone()), cfg)
    }

    #[test]
    fn promotion_preserves_home() {
        let (mut e, cfg) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("code.exe")));
        e.handle(Event::Opened(2, meta("windowsterminal.exe")));
        let term_home = e.home_of(2).unwrap();

        // Terminal promotes onto the stage at its fit, not full size.
        let cmds = e.handle(Event::Promoted(2));
        let stage = layout::window_rect(&cfg, layout::FOCAL);
        match &cmds[..] {
            [Command::Place { win: 2, to, .. }] => {
                assert!((to.w - stage.w * 0.55).abs() < 0.5);
                assert!((to.h - stage.h).abs() < 0.5);
            }
            other => panic!("unexpected commands: {other:?}"),
        }

        // Promoting the editor sends the terminal back to ITS home.
        let cmds = e.handle(Event::Promoted(1));
        let term_home_rect = layout::window_rect(&cfg, term_home);
        assert!(cmds.iter().any(
            |c| matches!(c, Command::Place { win: 2, to, .. } if *to == term_home_rect)
        ));
        assert_eq!(e.home_of(2), Some(term_home));
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn refocusing_the_focused_window_is_a_noop() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Promoted(1));
        // Clicking inside the focused window re-reports it as
        // foreground; nothing may move.
        assert!(e.handle(Event::Promoted(1)).is_empty());
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn set_screen_replaces_everyone() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Promoted(2));
        let cmds = e.set_screen(crate::config::Screen::from_px(3840, 2160, 65.0));
        assert_eq!(cmds.len(), 2, "every managed window is re-placed");
        // the focused window goes to the (new) focal stage
        let stage = layout::focal_rect(e.config(), Some(Fit { w: 1.0, h: 1.0 }));
        assert!(cmds.iter().any(|c| matches!(
            c, Command::Place { win: 2, to, .. } if (to.w - stage.w).abs() < 1.0)));
    }

    #[test]
    fn clear_stage_sends_focused_home() {
        let (mut e, cfg) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        let home = e.home_of(1).unwrap();
        e.handle(Event::Promoted(1));
        let cmds = e.handle(Event::ClearStage);
        assert_eq!(
            cmds,
            vec![Command::Place {
                win: 1,
                to: layout::window_rect(&cfg, home),
                animate: true
            }]
        );
        assert_eq!(e.focused(), None);
        // Second clear is a no-op.
        assert!(e.handle(Event::ClearStage).is_empty());
    }

    #[test]
    fn nothing_moves_while_suspended() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));

        e.handle(Event::Suspend(true));
        assert!(e.is_suspended());
        // The snip is in progress: nothing may move, for any reason.
        assert!(e.handle(Event::Promoted(2)).is_empty());
        assert!(e.handle(Event::ClearStage).is_empty());
        assert!(e.handle(Event::Opened(3, meta("c.exe"))).is_empty());
        assert_eq!(e.focused(), None);

        // Closing the overlay puts everything back where it belongs.
        let cmds = e.handle(Event::Suspend(false));
        assert_eq!(cmds.len(), 3, "every managed window is re-asserted");
        assert!(cmds.iter().all(|c| matches!(
            c, Command::Place { animate: false, .. })), "no animation on resume");
    }

    #[test]
    fn ignored_windows_are_never_adopted() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        let cmds = e.handle(Event::Opened(9, meta("SnippingTool.exe")));
        assert_eq!(cmds, vec![Command::Release(9)]);
        assert_eq!(e.home_of(9), None);
        // and it can never take the stage
        assert!(e.handle(Event::Promoted(9)).is_empty());
    }

    #[test]
    fn thirteenth_window_floats() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        for id in 1..=12u64 {
            let cmds = e.handle(Event::Opened(id, meta("app.exe")));
            assert!(matches!(cmds[..], [Command::Place { .. }]));
        }
        let cmds = e.handle(Event::Opened(13, meta("late.exe")));
        assert_eq!(cmds, vec![Command::Release(13)]);
    }

    #[test]
    fn undock_releases_everything() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        let cmds = e.handle(Event::DeskMode(false));
        assert_eq!(cmds.len(), 2);
        assert!(cmds.iter().all(|c| matches!(c, Command::Release(_))));
        // Inactive engine ignores promotion.
        assert!(e.handle(Event::Promoted(1)).is_empty());
    }

    #[test]
    fn reconfigure_rescales_without_rehoming() {
        let (mut e, cfg) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("windowsterminal.exe")));
        let (home1, home2) = (e.home_of(1).unwrap(), e.home_of(2).unwrap());
        e.handle(Event::Promoted(2));

        // Widening the gutter is a pure geometry change — the kind a
        // slider makes.
        let mut wider = cfg.clone();
        wider.gutter_in = cfg.gutter_in * 2.0;
        let cmds = e.handle(Event::Reconfigured(wider.clone()));

        assert_eq!(cmds.len(), 2, "every managed window is re-placed");
        // Who lives where does not change under a retune.
        assert_eq!(e.home_of(1), Some(home1));
        assert_eq!(e.home_of(2), Some(home2));
        assert_eq!(e.focused(), Some(2));
        // The unfocused window sits at its own home, under the NEW gutter.
        let home_rect = layout::window_rect(&wider, home1);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::Place { win: 1, to, .. } if *to == home_rect)),
            "window 1 should be re-placed at its home under the new gutter"
        );
        // The focused one stays on the stage, still at its fit.
        let stage = layout::focal_rect(&wider, Some(Fit { w: 0.55, h: 1.0 }));
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::Place { win: 2, to, .. } if *to == stage)),
            "window 2 should stay on the focal stage at its fit"
        );
    }

    #[test]
    fn closing_frees_the_slot() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        let s1 = e.home_of(1).unwrap();
        e.handle(Event::Closed(1));
        e.handle(Event::Opened(2, meta("b.exe")));
        assert_eq!(e.home_of(2), Some(s1), "freed slot should be reused first");
    }
}
