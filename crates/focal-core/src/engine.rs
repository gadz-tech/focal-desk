//! The promotion state machine. One event in, zero or more commands
//! out; no OS types anywhere. This is the file to read to understand
//! what focal-desk *does*.

use std::collections::{HashMap, HashSet};

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
    /// The user explicitly asked for this window on the stage, and the
    /// [`Source`] says how: a tap on its tab, a hotkey, a drop on the
    /// focal slot. Always promotes. Naming the source is what keeps a
    /// stray promotion from hiding — the adapter logs it, and nothing
    /// may construct this event without saying who asked
    /// (REQUESTS-2026-09-04 §1). (ARCHITECTURE: a new promotion gesture
    /// is adapter-only — it just sends this, with a new `Source` variant
    /// if none fits.)
    Promoted(WinId, Source),
    /// The OS says this window now holds the foreground — a click into
    /// its body, Alt+Tab, a taskbar click. **Informational only.** The
    /// engine records it (the adapter may style the active frame from
    /// it) and never places anything in response: with dwell off, being
    /// in a window is not a request to move it, and a copy in one side
    /// window followed by a paste in another must move nothing
    /// (REQUESTS-2026-09-04 §1, §2). The dwell timer, when it is on at
    /// all, still arrives separately as [`Event::Dwelled`].
    Foreground(WinId),
    /// A window held the foreground for the dwell period. (The adapter
    /// owns the timer; the engine only ever sees the decision point.)
    /// Promotes only while `dwell_ms > 0`: with dwell off, being *in* a
    /// window is not a request to move it (2026-09-03, REQUESTS-08-28 §2).
    Dwelled(WinId),
    /// The user dragged a window's tab onto a slot: re-home it there.
    /// The focal slot means "promote". (REQUESTS-2026-08-28 §3.)
    MoveTo(WinId, SlotId),
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

/// Who asked for a promotion. Every [`Event::Promoted`] carries one, so
/// the log can name it and a promotion from anywhere else is a compile
/// error rather than a mystery (REQUESTS-2026-09-04 §1: "the only sources
/// of `Promoted` are the tab tap, the hotkey and the drop on the focal
/// slot").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A tap on the window's tab (or frame).
    Tab,
    /// A promotion hotkey. (None is bound today; the variant exists so
    /// the next gesture names itself rather than borrowing `Tab`.)
    Hotkey,
    /// The window's tab was dragged and dropped on the focal slot.
    Drop,
    /// The dwell timer. Only ever reaches the engine as [`Event::Dwelled`],
    /// which the engine vetoes while `dwell_ms = 0`; the adapter has no
    /// reason to send `Promoted(_, Dwell)` and does not.
    Dwell,
}

impl Source {
    /// The word the log uses for this source.
    pub fn name(self) -> &'static str {
        match self {
            Source::Tab => "tab",
            Source::Hotkey => "hotkey",
            Source::Drop => "drop",
            Source::Dwell => "dwell",
        }
    }
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
    /// The window the OS last reported as foreground. Styling input only.
    foreground: Option<WinId>,
    /// The last promotion and who asked for it, for the log and the tests.
    last_promotion: Option<(WinId, Source)>,
    /// Windows adopted while the layout was frozen: they have a home but
    /// have never been placed, and get their one `Place` on resume.
    deferred: Vec<WinId>,
    /// Set when the screen or config changed while frozen; the resume then
    /// re-asserts everyone once, because every rect went stale.
    retuned_while_frozen: bool,
    /// Each managed window's slot preferences as of its adoption (its
    /// rule's `slots`, or empty), consulted when a priority claim
    /// displaces it (REQUESTS-2026-09-04 §8).
    prefs: HashMap<WinId, Vec<SlotId>>,
    /// Windows the user placed by hand (a tab drag to a slot). A rule —
    /// including another window's priority claim — never moves these;
    /// the hand wins until the window closes.
    pinned: HashSet<WinId>,
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
            foreground: None,
            last_promotion: None,
            deferred: Vec::new(),
            retuned_while_frozen: false,
            prefs: HashMap::new(),
            pinned: HashSet::new(),
        }
    }

    /// True when the user put this window where it is by hand.
    pub fn is_pinned(&self, id: WinId) -> bool {
        self.pinned.contains(&id)
    }

    /// The single entry point.
    pub fn handle(&mut self, ev: Event) -> Vec<Command> {
        match ev {
            Event::DeskMode(on) => self.set_active(on),
            Event::Opened(id, meta) => self.on_opened(id, meta),
            Event::Promoted(id, source) => self.on_promoted(id, source),
            Event::Foreground(id) => self.on_foreground(id),
            Event::Dwelled(id) => self.on_dwelled(id),
            Event::MoveTo(id, slot) => self.on_move_to(id, slot),
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
    /// The window the OS last reported as foreground, if any. Styling
    /// input, never placement input.
    pub fn foreground(&self) -> Option<WinId> {
        self.foreground
    }
    /// The most recent promotion and the gesture that asked for it.
    pub fn last_promotion(&self) -> Option<(WinId, Source)> {
        self.last_promotion
    }

    /// The rectangle the engine intends for a managed window right now:
    /// the stage (at its fit) while it is focused, else its home. This
    /// is where the adapter hangs the window's tap target.
    pub fn placement(&self, id: WinId) -> Option<Rect> {
        let slot = *self.homes.get(&id)?;
        Some(self.intended(id, slot))
    }

    /// Every window the engine has given a home, in no particular order.
    pub fn managed(&self) -> Vec<WinId> {
        self.homes.keys().copied().collect()
    }

    /// The window whose home is `slot`, if any (it may be on the stage).
    pub fn occupant(&self, slot: SlotId) -> Option<WinId> {
        self.occupants.get(&slot).copied()
    }

    /// Where `id` (whose home is `slot`) belongs under the current config.
    fn intended(&self, id: WinId, slot: SlotId) -> Rect {
        if self.focused == Some(id) {
            layout::focal_rect(&self.cfg, self.fits.get(&id).copied().flatten())
        } else {
            layout::window_rect(&self.cfg, slot)
        }
    }

    /// Freeze or resume. A resume places only what *changed* while frozen:
    /// windows adopted during the freeze get their first `Place`, and a
    /// retune (screen or config) that arrived while frozen re-asserts
    /// everyone once. A resume caused by nothing more than a foreground
    /// change — the common case: an overlay closed and the user clicked
    /// into a window — moves nothing. (Until 2026-09-04 every resume
    /// re-asserted every window, by design for captures; REQUESTS
    /// 2026-09-04 §2 retired that: a foreground change never re-places.)
    fn set_suspended(&mut self, on: bool) -> Vec<Command> {
        if on == self.suspended {
            return Vec::new();
        }
        self.suspended = on;
        if on || !self.active {
            return Vec::new();
        }
        let deferred = std::mem::take(&mut self.deferred);
        if std::mem::take(&mut self.retuned_while_frozen) {
            return self.reassert();
        }
        deferred
            .into_iter()
            .filter_map(|id| {
                let slot = *self.homes.get(&id)?;
                Some(Command::Place { win: id, to: self.intended(id, slot), animate: true })
            })
            .collect()
    }

    /// Record the OS foreground window and do nothing else. This is the
    /// engine's half of the §1/§2 invariant: whatever the adapter learns
    /// from the foreground hook, it cannot move a window through here.
    fn on_foreground(&mut self, id: WinId) -> Vec<Command> {
        self.foreground = Some(id);
        Vec::new()
    }

    /// A `Place` for every managed window at the position it should
    /// currently hold.
    fn reassert(&self) -> Vec<Command> {
        self.homes
            .iter()
            .map(|(&id, &slot)| Command::Place {
                win: id,
                to: self.intended(id, slot),
                animate: false,
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
    /// behind the change. An inactive engine commands nothing; a frozen
    /// one moves nothing under the overlay and settles up on resume.
    fn replace_all(&mut self) -> Vec<Command> {
        if !self.active {
            return Vec::new();
        }
        if self.suspended {
            self.retuned_while_frozen = true;
            return Vec::new();
        }
        self.reassert()
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
        // Whatever was owed from a freeze is settled by the flight below
        // (or moot once released).
        self.deferred.clear();
        self.retuned_while_frozen = false;
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

    /// Adopt a new window (REQUESTS-2026-09-04 §8). Its home is the first
    /// *free* slot in its rule's preference list; with `priority` it may
    /// instead claim an occupied preference, and the occupant moves to
    /// its own next preference (see [`Self::displace`]). A window the
    /// user placed by hand is never displaced — the claimant takes its
    /// next claimable preference or the fallback. No rule, or no free
    /// preference: first free slot center-out, as always. With no free
    /// slot at all the window stays unmanaged (floats).
    fn on_opened(&mut self, id: WinId, meta: WindowMeta) -> Vec<Command> {
        // Capture overlays and shell surfaces are left entirely alone.
        if self.cfg.is_ignored(&meta) {
            return vec![Command::Release(id)];
        }
        let rule = self.cfg.apps.iter().find(|r| r.matcher.matches(&meta));
        let fit = rule.and_then(|r| r.focal_fit);
        let prefs: Vec<SlotId> = rule.map(|r| r.slots.clone()).unwrap_or_default();
        let priority = rule.map(|r| r.priority).unwrap_or(false);

        let mut cmds = Vec::new();
        let mut slot = prefs.iter().copied().find(|s| !self.occupants.contains_key(s));
        if slot.is_none() && priority {
            // Claim the first preference whose occupant did not get there
            // by hand.
            let claim = prefs.iter().find_map(|&s| {
                self.occupants
                    .get(&s)
                    .copied()
                    .filter(|o| !self.pinned.contains(o))
                    .map(|o| (s, o))
            });
            if let Some((claimed, occupant)) = claim {
                cmds.extend(self.displace(occupant, claimed));
                slot = Some(claimed);
            }
        }
        let slot = slot.or_else(|| {
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
        self.prefs.insert(id, prefs);
        self.occupants.insert(slot, id);
        if self.active && !self.suspended {
            cmds.push(Command::Place {
                win: id,
                to: layout::window_rect(&self.cfg, slot),
                animate: true,
            });
        } else if self.active {
            // Adopted under an overlay: placed when it lifts.
            self.deferred.push(id);
        }
        cmds
    }

    /// Move `occupant` out of `slot`, which a priority rule is claiming:
    /// to its own next free preference, else the first free slot
    /// center-out; with nothing free anywhere it is released and floats
    /// — the 13th-window policy, applied to the loser of a claim. An
    /// occupant on the stage keeps the stage and only inherits the new
    /// home, exactly as in a tab-drag swap.
    fn displace(&mut self, occupant: WinId, slot: SlotId) -> Vec<Command> {
        self.occupants.remove(&slot);
        let own: Vec<SlotId> = self.prefs.get(&occupant).cloned().unwrap_or_default();
        let next = own
            .iter()
            .copied()
            .chain(layout::home_priority())
            .find(|s| *s != slot && !self.occupants.contains_key(s));
        match next {
            Some(next) => {
                self.homes.insert(occupant, next);
                self.occupants.insert(next, occupant);
                if self.focused == Some(occupant) || !self.active {
                    Vec::new()
                } else if self.suspended {
                    self.deferred.push(occupant);
                    Vec::new()
                } else {
                    vec![self.fly_home(occupant, next)]
                }
            }
            None => {
                self.homes.remove(&occupant);
                self.fits.remove(&occupant);
                self.prefs.remove(&occupant);
                self.pinned.remove(&occupant);
                self.deferred.retain(|&d| d != occupant);
                if self.focused == Some(occupant) {
                    self.focused = None;
                }
                vec![Command::Release(occupant)]
            }
        }
    }

    /// Put a window on the focal stage (shrunk to its fit hint) and
    /// send the previously focused window back to its own home. The
    /// source is recorded so the log and the tests can name who asked.
    fn on_promoted(&mut self, id: WinId, source: Source) -> Vec<Command> {
        if !self.active
            || self.suspended
            || !self.homes.contains_key(&id)
            || self.focused == Some(id)
        {
            return Vec::new();
        }
        self.last_promotion = Some((id, source));
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

    /// The dwell timer fired for a window the user is merely *in*. With
    /// dwell on this is the classic promotion; with it off it is nothing
    /// at all — the tab is the only promoter — so a plain click into a
    /// window body never moves anything, whatever the adapter reports.
    fn on_dwelled(&mut self, id: WinId) -> Vec<Command> {
        if !self.cfg.dwell_enabled() {
            return Vec::new();
        }
        self.on_promoted(id, Source::Dwell)
    }

    /// Manual placement: the user dragged a window's tab onto `slot`
    /// (REQUESTS-2026-08-28 §3). The focal slot is a promote. Any other
    /// slot becomes the window's new home — the one time a home changes,
    /// and it changes because the user said so. A slot that already
    /// belongs to another window **swaps the two homes**, so one drag
    /// fixes "Mail took the slot I wanted for Code"; the bystander flies
    /// to the mover's old slot (unless it is on the stage, in which case
    /// it only inherits the slot and lands there when displaced). To make
    /// an occupied slot refuse instead, change only the swap arm.
    fn on_move_to(&mut self, id: WinId, slot: SlotId) -> Vec<Command> {
        if !self.active || self.suspended || slot.0 as usize >= layout::SLOT_COUNT {
            return Vec::new();
        }
        let Some(&old) = self.homes.get(&id) else {
            return Vec::new();
        };
        if slot == layout::FOCAL {
            return self.on_promoted(id, Source::Drop);
        }
        let was_focused = self.focused == Some(id);
        if slot == old {
            // Its own home: nothing to do unless it is on the stage, in
            // which case the drop means "send it home".
            if !was_focused {
                return Vec::new();
            }
            self.focused = None;
            return vec![self.fly_home(id, old)];
        }
        let mut cmds = Vec::new();
        match self.occupants.get(&slot).copied() {
            Some(other) => {
                // The swap arm.
                self.homes.insert(other, old);
                self.occupants.insert(old, other);
                if self.focused != Some(other) {
                    cmds.push(self.fly_home(other, old));
                }
            }
            None => {
                self.occupants.remove(&old);
            }
        }
        self.homes.insert(id, slot);
        self.occupants.insert(slot, id);
        // The hand wins over every rule for this window until it closes
        // (§8); the bystander in a swap did not ask and is not pinned.
        self.pinned.insert(id);
        if was_focused {
            self.focused = None;
        }
        cmds.push(self.fly_home(id, slot));
        cmds
    }

    /// The animated `Place` that flies `id` to `slot`'s window rect.
    fn fly_home(&self, id: WinId, slot: SlotId) -> Command {
        Command::Place {
            win: id,
            to: layout::window_rect(&self.cfg, slot),
            animate: true,
        }
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
        self.prefs.remove(&id);
        self.pinned.remove(&id);
        self.deferred.retain(|&d| d != id);
        if self.focused == Some(id) {
            self.focused = None;
        }
        if self.foreground == Some(id) {
            self.foreground = None;
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
        WindowMeta { process: p.into(), title: String::new(), class: String::new() }
    }

    /// An engine whose config gives terminals a 0.55-wide focal fit.
    fn engine_with_terminal_rule() -> (Engine, Config) {
        let mut cfg = Config::default();
        cfg.apps = vec![AppRule {
            matcher: Matcher::Process("*terminal*".into()),
            slots: Vec::new(),
            priority: false,
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
        let cmds = e.handle(Event::Promoted(2, Source::Tab));
        let stage = layout::window_rect(&cfg, layout::FOCAL);
        match &cmds[..] {
            [Command::Place { win: 2, to, .. }] => {
                assert!((to.w - stage.w * 0.55).abs() < 0.5);
                assert!((to.h - stage.h).abs() < 0.5);
            }
            other => panic!("unexpected commands: {other:?}"),
        }

        // Promoting the editor sends the terminal back to ITS home.
        let cmds = e.handle(Event::Promoted(1, Source::Tab));
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
        e.handle(Event::Promoted(1, Source::Tab));
        // Clicking inside the focused window re-reports it as
        // foreground; nothing may move.
        assert!(e.handle(Event::Promoted(1, Source::Tab)).is_empty());
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn set_screen_replaces_everyone() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Promoted(2, Source::Tab));
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
        e.handle(Event::Promoted(1, Source::Tab));
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
        assert!(e.handle(Event::Promoted(2, Source::Tab)).is_empty());
        assert!(e.handle(Event::ClearStage).is_empty());
        assert!(e.handle(Event::Opened(3, meta("c.exe"))).is_empty());
        assert_eq!(e.focused(), None);

        // Closing the overlay places the window that arrived during it;
        // the two already in place are not touched (2026-09-04 §2: a
        // resume is not a re-assert — until then all three were snapped).
        let cmds = e.handle(Event::Suspend(false));
        assert_eq!(cmds.len(), 1, "only the newcomer is placed");
        assert!(matches!(cmds[0], Command::Place { win: 3, animate: true, .. }));
    }

    #[test]
    fn ignored_windows_are_never_adopted() {
        let (mut e, _) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        let cmds = e.handle(Event::Opened(9, meta("SnippingTool.exe")));
        assert_eq!(cmds, vec![Command::Release(9)]);
        assert_eq!(e.home_of(9), None);
        // and it can never take the stage
        assert!(e.handle(Event::Promoted(9, Source::Tab)).is_empty());
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
        assert!(e.handle(Event::Promoted(1, Source::Tab)).is_empty());
    }

    #[test]
    fn reconfigure_rescales_without_rehoming() {
        let (mut e, cfg) = engine_with_terminal_rule();
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("windowsterminal.exe")));
        let (home1, home2) = (e.home_of(1).unwrap(), e.home_of(2).unwrap());
        e.handle(Event::Promoted(2, Source::Tab));

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

    // ---- the tab and the drag (2026-09-03) ----------------------------

    /// An engine in desk mode with dwell turned off: the tab is the only
    /// way onto the stage. Same terminal rule as the others.
    fn tab_only_engine() -> (Engine, Config) {
        let (_, mut cfg) = engine_with_terminal_rule();
        cfg.dwell_ms = 0;
        let mut e = Engine::new(cfg.clone());
        e.handle(Event::DeskMode(true));
        (e, cfg)
    }

    /// The animated `Place` that puts `id` at `slot`'s window rect.
    fn at_home(cfg: &Config, id: WinId, slot: SlotId) -> Command {
        Command::Place { win: id, to: layout::window_rect(cfg, slot), animate: true }
    }

    #[test]
    fn with_dwell_off_a_body_click_never_promotes() {
        let (mut e, _) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        // The adapter should never even start the timer, but the engine
        // has the last word: a dwell report moves nothing.
        assert!(e.handle(Event::Dwelled(1)).is_empty());
        assert_eq!(e.focused(), None);
    }

    #[test]
    fn a_tab_tap_promotes_with_dwell_off() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        let cmds = e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(
            cmds,
            vec![Command::Place { win: 1, to: layout::focal_rect(&cfg, None), animate: true }]
        );
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn dwell_still_promotes_while_left_on() {
        // The fallback: dwell switched back on in the config (the shipped
        // default is 0 since the 2026-09-03 live run).
        let (_, mut cfg) = engine_with_terminal_rule();
        cfg.dwell_ms = 1200;
        let mut e = Engine::new(cfg);
        e.handle(Event::DeskMode(true));
        e.handle(Event::Opened(1, meta("a.exe")));
        assert!(matches!(e.handle(Event::Dwelled(1))[..], [Command::Place { win: 1, .. }]));
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn desktop_click_still_clears_a_tab_promote() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        let home = e.home_of(1).unwrap();
        e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(e.handle(Event::ClearStage), vec![at_home(&cfg, 1, home)]);
        assert_eq!(e.focused(), None);
    }

    #[test]
    fn move_to_a_free_slot_replaces_only_that_window() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Opened(3, meta("c.exe")));
        let (h1, h2, h3) = (e.home_of(1).unwrap(), e.home_of(2).unwrap(), e.home_of(3).unwrap());
        let target = layout::slot_from_name("corner-br").unwrap();
        assert_eq!(e.occupant(target), None);

        let cmds = e.handle(Event::MoveTo(2, target));
        assert_eq!(cmds, vec![at_home(&cfg, 2, target)], "exactly one window moves");
        assert_eq!(e.home_of(2), Some(target));
        assert_eq!(e.occupant(target), Some(2));
        assert_eq!(e.occupant(h2), None, "the old slot is free again");
        // Nobody else was touched.
        assert_eq!(e.home_of(1), Some(h1));
        assert_eq!(e.home_of(3), Some(h3));
        assert_eq!(e.focused(), None);
    }

    #[test]
    fn move_to_the_focal_slot_is_a_promote() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        let (h1, h2) = (e.home_of(1).unwrap(), e.home_of(2).unwrap());
        e.handle(Event::Promoted(1, Source::Tab));
        // Exactly what a tap on 2's tab would do: 1 goes home, 2 takes the stage.
        let cmds = e.handle(Event::MoveTo(2, layout::FOCAL));
        assert_eq!(
            cmds,
            vec![
                at_home(&cfg, 1, h1),
                Command::Place { win: 2, to: layout::focal_rect(&cfg, None), animate: true },
            ]
        );
        assert_eq!(e.focused(), Some(2));
        assert_eq!(e.home_of(2), Some(h2), "a promote never re-homes");
    }

    #[test]
    fn move_to_an_occupied_slot_swaps_the_two_homes() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Opened(3, meta("c.exe")));
        let (h1, h2, h3) = (e.home_of(1).unwrap(), e.home_of(2).unwrap(), e.home_of(3).unwrap());

        let cmds = e.handle(Event::MoveTo(1, h2));
        assert_eq!(cmds.len(), 2, "the mover and the one it displaced");
        assert!(cmds.contains(&at_home(&cfg, 1, h2)));
        assert!(cmds.contains(&at_home(&cfg, 2, h1)));
        assert_eq!(e.home_of(1), Some(h2));
        assert_eq!(e.home_of(2), Some(h1));
        assert_eq!(e.occupant(h1), Some(2));
        assert_eq!(e.occupant(h2), Some(1));
        assert_eq!(e.home_of(3), Some(h3), "the bystander keeps its home");
    }

    #[test]
    fn moving_the_focused_window_to_a_slot_leaves_the_stage() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Promoted(1, Source::Tab));
        let target = layout::slot_from_name("right-bottom").unwrap();
        assert_eq!(e.handle(Event::MoveTo(1, target)), vec![at_home(&cfg, 1, target)]);
        assert_eq!(e.focused(), None);
        assert_eq!(e.home_of(1), Some(target));
        // At home already: a drop on its own slot is nothing.
        assert!(e.handle(Event::MoveTo(1, target)).is_empty());
        // From the stage, a drop on its own home is "send it home".
        e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(e.handle(Event::MoveTo(1, target)), vec![at_home(&cfg, 1, target)]);
        assert_eq!(e.focused(), None);
    }

    #[test]
    fn a_drop_on_the_focused_windows_home_takes_it_over() {
        // 1 is on the stage, so its home slot looks empty. Dropping 2
        // there swaps homes, but only 2 moves: 1 keeps the stage and
        // lands in its new home when it is displaced.
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        let (h1, h2) = (e.home_of(1).unwrap(), e.home_of(2).unwrap());
        e.handle(Event::Promoted(1, Source::Tab));

        assert_eq!(e.handle(Event::MoveTo(2, h1)), vec![at_home(&cfg, 2, h1)]);
        assert_eq!(e.focused(), Some(1));
        assert_eq!(e.home_of(1), Some(h2));
        assert_eq!(e.home_of(2), Some(h1));
        // Displacing 1 now sends it to its new home.
        let cmds = e.handle(Event::Promoted(2, Source::Tab));
        assert!(cmds.contains(&at_home(&cfg, 1, h2)));
    }

    #[test]
    fn move_to_ignores_strangers_frozen_layouts_and_bad_slots() {
        let (mut e, _) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        let target = layout::slot_from_name("corner-tl").unwrap();
        assert!(e.handle(Event::MoveTo(99, target)).is_empty(), "unknown window");
        assert!(e.handle(Event::MoveTo(1, SlotId(200))).is_empty(), "no such slot");
        e.handle(Event::Suspend(true));
        assert!(e.handle(Event::MoveTo(1, target)).is_empty(), "frozen for an overlay");
        e.handle(Event::Suspend(false));
        e.handle(Event::DeskMode(false));
        assert!(e.handle(Event::MoveTo(1, target)).is_empty(), "laptop mode");
        assert_eq!(e.home_of(1), Some(SlotId(1)), "nothing above changed a home");
    }

    // ---- focus comes from the tab only (2026-09-04, REQUESTS §1/§2) ----

    /// Every command the engine emits for a sequence of events, flattened.
    fn all_commands(e: &mut Engine, events: Vec<Event>) -> Vec<Command> {
        events.into_iter().flat_map(|ev| e.handle(ev)).collect()
    }

    #[test]
    fn a_foreground_change_never_places_anything() {
        // Dwell off and dwell on; a window on the stage, one at home, one
        // unknown: the OS foreground hook can say what it likes and
        // nothing moves, nothing changes hands.
        for dwell in [0u64, 1200] {
            let (_, mut cfg) = engine_with_terminal_rule();
            cfg.dwell_ms = dwell;
            let mut e = Engine::new(cfg);
            e.handle(Event::DeskMode(true));
            e.handle(Event::Opened(1, meta("a.exe")));
            e.handle(Event::Opened(2, meta("b.exe")));
            e.handle(Event::Promoted(1, Source::Tab));
            let cmds = all_commands(
                &mut e,
                vec![
                    Event::Foreground(2),
                    Event::Foreground(1),
                    Event::Foreground(2),
                    Event::Foreground(77),
                ],
            );
            assert!(cmds.is_empty(), "dwell {dwell}: foreground changes emitted {cmds:?}");
            assert_eq!(e.focused(), Some(1), "dwell {dwell}: the stage did not change hands");
            assert_eq!(e.foreground(), Some(77), "the foreground is recorded for styling");
        }
    }

    #[test]
    fn a_body_click_after_an_overlay_moves_nothing() {
        // The copy-paste case with an overlay in between: Win+V (clipboard
        // history) freezes the layout, the click into window B resumes
        // it. Neither the resume nor the foreground change may re-place a
        // window that already has its place.
        let (mut e, _) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Promoted(1, Source::Tab));
        assert!(e.handle(Event::Suspend(true)).is_empty());
        let cmds = all_commands(&mut e, vec![Event::Suspend(false), Event::Foreground(2)]);
        assert!(cmds.is_empty(), "resume + foreground emitted {cmds:?}");
        assert_eq!(e.focused(), Some(1));
    }

    #[test]
    fn a_resume_places_only_what_arrived_while_frozen() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Suspend(true));
        assert!(e.handle(Event::Opened(2, meta("b.exe"))).is_empty());
        let home2 = e.home_of(2).unwrap();
        let cmds = e.handle(Event::Suspend(false));
        assert_eq!(cmds, vec![at_home(&cfg, 2, home2)], "only the newcomer is placed");
        // A second freeze/resume has nothing left to do.
        e.handle(Event::Suspend(true));
        assert!(e.handle(Event::Suspend(false)).is_empty());
        // A newcomer that closes before the resume is not placed either.
        e.handle(Event::Suspend(true));
        e.handle(Event::Opened(3, meta("c.exe")));
        e.handle(Event::Closed(3));
        assert!(e.handle(Event::Suspend(false)).is_empty());
    }

    #[test]
    fn a_retune_while_frozen_waits_for_the_resume() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        e.handle(Event::Suspend(true));
        let mut wider = cfg.clone();
        wider.gutter_in = cfg.gutter_in * 2.0;
        assert!(
            e.handle(Event::Reconfigured(wider.clone())).is_empty(),
            "nothing moves under an overlay, not even for a retune"
        );
        assert!(e.set_screen(crate::config::Screen::from_px(3840, 2160, 65.0)).is_empty());
        let cmds = e.handle(Event::Suspend(false));
        assert_eq!(cmds.len(), 2, "every window is re-asserted under the new geometry");
        assert!(cmds.iter().all(|c| matches!(c, Command::Place { animate: false, .. })));
        let home1 = e.home_of(1).unwrap();
        assert!(cmds.contains(&Command::Place {
            win: 1,
            to: layout::window_rect(e.config(), home1),
            animate: false
        }));
    }

    // ---- assignable slots (2026-09-04, REQUESTS §8) ----------------------

    /// A rule: `matcher` → ordered `slots` by name, with or without priority.
    fn rule(matcher: Matcher, slots: &[&str], priority: bool) -> AppRule {
        AppRule {
            matcher,
            slots: slots.iter().map(|s| slot(s)).collect(),
            priority,
            focal_fit: None,
        }
    }

    /// The slot named in a config file, unwrapped for the tests.
    fn slot(name: &str) -> SlotId {
        layout::slot_from_name(name).unwrap()
    }

    /// A tab-only engine in desk mode running these rules.
    fn engine_with_rules(rules: Vec<AppRule>) -> (Engine, Config) {
        let mut cfg = Config::default();
        cfg.dwell_ms = 0;
        cfg.apps = rules;
        let mut e = Engine::new(cfg.clone());
        e.handle(Event::DeskMode(true));
        (e, cfg)
    }

    #[test]
    fn a_rule_gives_the_first_free_preferred_slot() {
        let corners = ["corner-tl", "corner-tr", "corner-bl", "corner-br"];
        let (mut e, _) =
            engine_with_rules(vec![rule(Matcher::Process("*terminal*".into()), &corners, false)]);
        e.handle(Event::Opened(1, meta("WindowsTerminal.exe")));
        e.handle(Event::Opened(2, meta("WindowsTerminal.exe")));
        assert_eq!(e.home_of(1), Some(slot("corner-tl")));
        assert_eq!(e.home_of(2), Some(slot("corner-tr")));
        // A stranger sitting in a preferred slot is skipped over, not moved.
        e.handle(Event::Opened(3, meta("a.exe")));
        assert_eq!(e.home_of(3), Some(slot("left-top")), "no rule: center-out");
        e.handle(Event::MoveTo(3, slot("corner-bl")));
        e.handle(Event::Opened(4, meta("WindowsTerminal.exe")));
        assert_eq!(e.home_of(4), Some(slot("corner-br")));
        assert_eq!(e.home_of(3), Some(slot("corner-bl")));
        // Every preference taken: today's fallback, first free center-out.
        e.handle(Event::Opened(5, meta("WindowsTerminal.exe")));
        assert_eq!(e.home_of(5), Some(slot("left-top")));
    }

    #[test]
    fn a_priority_claim_moves_the_occupant_to_its_next_preference() {
        let (mut e, cfg) = engine_with_rules(vec![
            rule(Matcher::Substring("household".into()), &["top-1"], true),
            rule(Matcher::Process("*olk*".into()), &["top-1", "top-2", "bottom-1"], false),
        ]);
        e.handle(Event::Opened(1, meta("olk.exe")));
        assert_eq!(e.home_of(1), Some(slot("top-1")));
        let kiosk = WindowMeta {
            process: "msedge.exe".into(),
            title: "Household — kiosk".into(),
            class: String::new(),
        };
        let cmds = e.handle(Event::Opened(2, kiosk));
        assert_eq!(e.home_of(2), Some(slot("top-1")), "the priority window took the slot");
        assert_eq!(e.home_of(1), Some(slot("top-2")), "mail moved to its own next preference");
        assert_eq!(cmds, vec![at_home(&cfg, 1, slot("top-2")), at_home(&cfg, 2, slot("top-1"))]);
        assert_eq!(e.occupant(slot("top-1")), Some(2));
        assert_eq!(e.occupant(slot("top-2")), Some(1));
        // Without priority the same rule would simply have taken top-2.
        let (mut e, _) = engine_with_rules(vec![
            rule(Matcher::Substring("household".into()), &["top-1", "top-2"], false),
            rule(Matcher::Process("*olk*".into()), &["top-1"], false),
        ]);
        e.handle(Event::Opened(1, meta("olk.exe")));
        e.handle(Event::Opened(2, meta("household.exe")));
        assert_eq!(e.home_of(1), Some(slot("top-1")));
        assert_eq!(e.home_of(2), Some(slot("top-2")));
        // An occupant with no rule of its own falls back center-out.
        let (mut e, _) = engine_with_rules(vec![rule(
            Matcher::Substring("household".into()),
            &["left-top"],
            true,
        )]);
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("household.exe")));
        assert_eq!(e.home_of(2), Some(slot("left-top")));
        assert_eq!(e.home_of(1), Some(slot("left-bottom")));
    }

    #[test]
    fn a_hand_move_beats_a_rule_until_the_window_closes() {
        let (mut e, cfg) = engine_with_rules(vec![
            rule(Matcher::Substring("household".into()), &["top-2"], true),
            rule(Matcher::Process("*olk*".into()), &["top-1", "top-2"], false),
        ]);
        e.handle(Event::Opened(1, meta("olk.exe")));
        e.handle(Event::MoveTo(1, slot("top-2")));
        assert_eq!(e.home_of(1), Some(slot("top-2")));
        assert!(e.is_pinned(1));
        // A later priority claim cannot displace a hand-placed window…
        e.handle(Event::Opened(2, meta("household.exe")));
        assert_eq!(e.home_of(1), Some(slot("top-2")), "the hand placement stands");
        assert_eq!(e.home_of(2), Some(slot("left-top")), "the claimant takes the fallback");
        // …and a later retune re-asserts the hand placement, not the rule's.
        let cmds = e.handle(Event::Reconfigured(cfg.clone()));
        assert!(cmds.contains(&Command::Place {
            win: 1,
            to: layout::window_rect(&cfg, slot("top-2")),
            animate: false
        }));
        // Once it closes, the slot is a rule's to take again.
        e.handle(Event::Closed(1));
        assert!(!e.is_pinned(1));
        e.handle(Event::Opened(3, meta("olk.exe")));
        assert_eq!(e.home_of(3), Some(slot("top-1")));
        e.handle(Event::Opened(4, meta("household.exe")));
        assert_eq!(e.home_of(4), Some(slot("top-2")));
    }

    #[test]
    fn no_rule_means_todays_placement() {
        let (mut e, _) = engine_with_rules(vec![rule(
            Matcher::Process("*terminal*".into()),
            &["corner-tl"],
            true,
        )]);
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        assert_eq!(e.home_of(1), Some(SlotId(1)), "first free, center-out");
        assert_eq!(e.home_of(2), Some(SlotId(2)));
        assert!(!e.is_pinned(1) && !e.is_pinned(2));
    }

    #[test]
    fn a_claim_with_nothing_free_floats_the_loser() {
        let (mut e, _) = engine_with_rules(vec![rule(
            Matcher::Substring("household".into()),
            &["left-top"],
            true,
        )]);
        for id in 1..=12u64 {
            e.handle(Event::Opened(id, meta("app.exe")));
        }
        let cmds = e.handle(Event::Opened(13, meta("household.exe")));
        assert_eq!(e.home_of(13), Some(slot("left-top")));
        assert_eq!(e.home_of(1), None, "the displaced window has nowhere to go");
        assert!(cmds.contains(&Command::Release(1)));
        assert!(matches!(cmds[..], [Command::Release(1), Command::Place { win: 13, .. }]));
    }

    #[test]
    fn a_move_emits_exactly_one_final_place() {
        // §9: the flight is the adapter's business. The engine says where
        // a window ends up — once, the final rectangle — and never an
        // intermediate one; the adapter animates a snapshot toward it and
        // resizes the real window a single time.
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        let target = layout::slot_from_name("corner-br").unwrap();
        assert_eq!(e.handle(Event::MoveTo(1, target)), vec![at_home(&cfg, 1, target)]);
        // A promote is the same: one final Place per window that moves.
        assert_eq!(
            e.handle(Event::Promoted(2, Source::Tab)),
            vec![Command::Place { win: 2, to: layout::focal_rect(&cfg, None), animate: true }]
        );
        let cmds = e.handle(Event::Promoted(1, Source::Tab));
        let for_1: Vec<&Command> =
            cmds.iter().filter(|c| matches!(c, Command::Place { win: 1, .. })).collect();
        assert_eq!(for_1.len(), 1, "the mover gets exactly one Place");
        assert_eq!(
            for_1[0],
            &Command::Place { win: 1, to: layout::focal_rect(&cfg, None), animate: true }
        );
        let h2 = e.home_of(2).unwrap();
        assert_eq!(cmds.len(), 2, "…and the displaced window gets exactly one");
        assert!(cmds.contains(&at_home(&cfg, 2, h2)));
    }

    #[test]
    fn every_promotion_names_its_source() {
        let (mut e, _) = tab_only_engine();
        e.handle(Event::Opened(1, meta("a.exe")));
        e.handle(Event::Opened(2, meta("b.exe")));
        assert_eq!(e.last_promotion(), None);
        e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(e.last_promotion(), Some((1, Source::Tab)));
        e.handle(Event::MoveTo(2, layout::FOCAL));
        assert_eq!(e.last_promotion(), Some((2, Source::Drop)));
        e.handle(Event::Promoted(1, Source::Hotkey));
        assert_eq!(e.last_promotion(), Some((1, Source::Hotkey)));
        // A refused promotion (already on the stage) is not a promotion.
        e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(e.last_promotion(), Some((1, Source::Hotkey)));
        // Dwell, while it is on, names itself too.
        let (_, mut cfg) = engine_with_terminal_rule();
        cfg.dwell_ms = 1200;
        let mut d = Engine::new(cfg);
        d.handle(Event::DeskMode(true));
        d.handle(Event::Opened(1, meta("a.exe")));
        d.handle(Event::Dwelled(1));
        assert_eq!(d.last_promotion(), Some((1, Source::Dwell)));
        assert_eq!(Source::Dwell.name(), "dwell");
    }

    #[test]
    fn placement_follows_focus_and_homes() {
        let (mut e, cfg) = tab_only_engine();
        e.handle(Event::Opened(1, meta("windowsterminal.exe")));
        let home = e.home_of(1).unwrap();
        assert_eq!(e.placement(1), Some(layout::window_rect(&cfg, home)));
        e.handle(Event::Promoted(1, Source::Tab));
        assert_eq!(
            e.placement(1),
            Some(layout::focal_rect(&cfg, Some(Fit { w: 0.55, h: 1.0 })))
        );
        assert_eq!(e.placement(7), None, "not managed");
        assert_eq!(e.managed(), vec![1]);
    }
}
