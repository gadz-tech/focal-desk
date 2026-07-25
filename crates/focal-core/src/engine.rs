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
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            active: false,
            homes: HashMap::new(),
            fits: HashMap::new(),
            occupants: HashMap::new(),
            focused: None,
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
        }
    }

    pub fn focused(&self) -> Option<WinId> {
        self.focused
    }
    pub fn home_of(&self, id: WinId) -> Option<SlotId> {
        self.homes.get(&id).copied()
    }
    pub fn is_active(&self) -> bool {
        self.active
    }

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

    fn on_opened(&mut self, id: WinId, meta: WindowMeta) -> Vec<Command> {
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
        if self.active {
            vec![Command::Place {
                win: id,
                to: layout::window_rect(&self.cfg, slot),
                animate: true,
            }]
        } else {
            Vec::new()
        }
    }

    fn on_promoted(&mut self, id: WinId) -> Vec<Command> {
        if !self.active || !self.homes.contains_key(&id) || self.focused == Some(id) {
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

    fn on_clear_stage(&mut self) -> Vec<Command> {
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

    fn meta(p: &str) -> WindowMeta {
        WindowMeta { process: p.into(), title: String::new() }
    }

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
