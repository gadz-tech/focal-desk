//! Configuration model. Plain Rust for now; serde/TOML loading is a
//! planned addition once dependency fetching is set up — the shape is
//! already serialization-friendly (owned strings, no lifetimes).

use crate::layout::SlotId;

#[derive(Clone, Debug)]
pub struct Screen {
    pub px_w: u32,
    pub px_h: u32,
    /// Physical width of the panel. Lets config talk in inches
    /// ("gutter: 1.5in") and stay meaningful across resolutions.
    pub inches_w: f32,
}

impl Screen {
    /// Pixel density: horizontal pixels over physical width.
    pub fn px_per_inch(&self) -> f32 {
        self.px_w as f32 / self.inches_w
    }
    /// The target display: 65" 16:9 at 8K (~135.6 ppi).
    pub fn desk_65_8k() -> Self {
        Self { px_w: 7680, px_h: 4320, inches_w: 56.65 }
    }

    /// Build from a live display mode plus the panel's diagonal size:
    /// physical width comes from the diagonal and the pixel aspect
    /// ratio, so the inch-based gutter is right on any screen.
    pub fn from_px(px_w: u32, px_h: u32, diagonal_in: f32) -> Self {
        let (w, h) = (px_w as f32, px_h as f32);
        let inches_w = diagonal_in * w / (w * w + h * h).sqrt();
        Self { px_w, px_h, inches_w }
    }
}

/// How a window occupies the focal stage. `1.0 x 1.0` fills it; a
/// terminal wants something like `0.55 x 1.0` (a tall column).
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub w: f32,
    pub h: f32,
}

impl Default for Fit {
    /// The default fit fills the stage exactly.
    fn default() -> Self {
        Self { w: 1.0, h: 1.0 }
    }
}

/// The shape of the tap target beside every managed window (Ryan,
/// 2026-08-28: "a border around each window … all the way around, or
/// maybe a tab that is always facing the center of the screen"). Both
/// are drawn by the adapter from `tab.rs` geometry; which one is a
/// matter of feel, so it is config, not code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapStyle {
    /// One tab on the edge that faces the screen center.
    Tab,
    /// A ring around the whole window, `tab_in` wide.
    Border,
}

impl TapStyle {
    /// Parse the config spelling: `tab` or `border`, case-insensitive.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "tab" => Some(TapStyle::Tab),
            "border" => Some(TapStyle::Border),
            _ => None,
        }
    }
}

/// What the adapter reports about a window when it appears.
#[derive(Clone, Debug, Default)]
pub struct WindowMeta {
    pub process: String,
    pub title: String,
    /// The window class name (`Chrome_WidgetWin_1`,
    /// `CASCADIA_HOSTING_WINDOW_CLASS`, …), matchable since 2026-09-04.
    pub class: String,
}

/// Matches windows to rules. Globs are case-insensitive, `*` wildcard.
#[derive(Clone, Debug)]
pub enum Matcher {
    Process(String),
    Title(String),
    /// The window class name, as a glob.
    Class(String),
    /// A case-insensitive substring of the exe name, the class or the
    /// title — the `match =` key (REQUESTS-2026-09-04 §8):
    /// `match = household` finds the kiosk wherever it shows up.
    Substring(String),
    Any,
}

impl Matcher {
    /// True when this matcher's glob matches the relevant window field
    /// (any of the three, for a substring).
    pub fn matches(&self, meta: &WindowMeta) -> bool {
        match self {
            Matcher::Process(g) => glob_match(g, &meta.process),
            Matcher::Title(g) => glob_match(g, &meta.title),
            Matcher::Class(g) => glob_match(g, &meta.class),
            Matcher::Substring(s) => {
                let g = format!("*{s}*");
                glob_match(&g, &meta.process)
                    || glob_match(&g, &meta.class)
                    || glob_match(&g, &meta.title)
            }
            Matcher::Any => true,
        }
    }
}

/// Classic iterative wildcard match, case-insensitive.
fn glob_match(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Per-app placement rule. First matching rule wins.
#[derive(Clone, Debug)]
pub struct AppRule {
    pub matcher: Matcher,
    /// Home slots in order of preference: the first free one is taken
    /// (`slots = corners`, `slots = top-1, top-2`; `home = X` is the
    /// one-slot spelling). Empty means "first free, center-out"
    /// (REQUESTS-2026-09-04 §8).
    pub slots: Vec<SlotId>,
    /// Claim the first preference even if it is occupied; the occupant
    /// moves to its own next preference (or the fallback). A window the
    /// user placed by hand is never displaced.
    pub priority: bool,
    /// How this app sits on the focal stage when promoted.
    pub focal_fit: Option<Fit>,
}

/// A hard (auto-detected) wire between two window populations. Soft
/// wires are runtime state (user-drawn), not config.
#[derive(Clone, Debug)]
pub struct WireRule {
    pub a: Matcher,
    pub b: Matcher,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub screen: Screen,
    /// The structural gutter, in inches. It actively sizes windows:
    /// every window is its region inset by half a gutter, so the gutter
    /// is exact everywhere and belongs solely to the wires.
    pub gutter_in: f32,
    /// Width of the focal column as a fraction of the screen.
    pub focal_frac: f32,
    /// Height of the top and bottom bands as a fraction of the screen.
    pub band_frac: f32,
    /// How long a window must hold the foreground before promotion, in
    /// milliseconds. **`0` turns dwell off**: then only an explicit
    /// gesture promotes — a tap on the window's tab, a hotkey — and a
    /// plain click into a window body never moves anything (Ryan,
    /// 2026-08-28: the 1.35 s delay). Off by default since the first live
    /// run (Ryan, 2026-09-03: the dwell timer *was* the sluggishness);
    /// non-zero brings hold-to-promote back as a fallback.
    pub dwell_ms: u64,
    /// Tab on the center-facing edge, or a border all the way around.
    pub tap_style: TapStyle,
    /// Depth of the tab (or width of the border), in inches — a physical
    /// quantity like the gutter, so it survives a resolution change. It
    /// is clamped to half the gutter at layout time: a tap target never
    /// leaves its own window's side of the channel, so targets never
    /// overlap each other or any window.
    pub tab_in: f32,
    /// Length of the tab along its edge, in inches (clamped to the edge).
    pub tab_length_in: f32,
    /// Width of the slim sides of a window's frame, in inches — the
    /// three sides that do not face the screen center (2026-09-04,
    /// REQUESTS §3). Clamped to half the gutter like `tab_in`.
    pub frame_in: f32,
    /// Width of the stage's frame, in inches: slim all round, no thick
    /// side, "just thick enough for the landing holes" (REQUESTS
    /// 2026-09-04 §3 rev 4, §6). Clamped to half the gutter.
    pub stage_in: f32,
    /// Physical diagonal of the desk panel, used to convert the
    /// inch-based gutter into pixels once the mode is known.
    pub screen_diagonal_in: f32,
    /// Manage windows even when the desk display isn't detected.
    /// Handy for trying the layout on a laptop screen.
    pub force_active: bool,
    pub apps: Vec<AppRule>,
    pub wires: Vec<WireRule>,
    /// Windows that focal-desk must not touch. While one of these holds
    /// the foreground the whole layout freezes, because these are the
    /// overlays you are *doing something with* — screen capture, the
    /// task switcher — and a window sliding underneath ruins the shot.
    pub ignore: Vec<Matcher>,
    /// Problems the parser stepped over: one line per `[app]` rule it
    /// skipped, naming the line and the reason. The adapter logs them; a
    /// typo in one rule never costs the others (2026-09-04 §8).
    pub warnings: Vec<String>,
}

impl Default for Config {
    /// The setup this project is built around: 65" 8K, 1.5" gutter,
    /// 56% focal column, 22% bands, dwell off (tab-only, since the
    /// 2026-09-03 live run), a 0.75" × 5" tab, 0.15" slim frame sides
    /// and a 0.35" stage ring.
    fn default() -> Self {
        Self {
            screen: Screen::desk_65_8k(),
            gutter_in: 1.5,
            focal_frac: 0.56,
            band_frac: 0.22,
            dwell_ms: 0,
            tap_style: TapStyle::Tab,
            tab_in: 0.75,
            tab_length_in: 5.0,
            frame_in: 0.15,
            stage_in: 0.35,
            screen_diagonal_in: 65.0,
            force_active: false,
            apps: Vec::new(),
            wires: Vec::new(),
            ignore: default_ignores(),
            warnings: Vec::new(),
        }
    }
}

/// Shell surfaces and capture overlays that are never managed, and that
/// freeze the layout while they are in front. Snipping Tool is the one
/// that matters day to day: it goes fullscreen and transparent over
/// everything, and any window motion underneath lands in the capture.
pub fn default_ignores() -> Vec<Matcher> {
    [
        "*snippingtool*",
        "*screenclippinghost*",
        "*screensketch*",
        "*textinputhost*",
        "*searchhost*",
        "*startmenuexperiencehost*",
        "*shellexperiencehost*",
        "*peopleexperiencehost*",
        "*lockapp*",
        "*magnify*",
    ]
    .into_iter()
    .map(|g| Matcher::Process(g.to_string()))
    .collect()
}

impl Config {
    /// True when this window must be left alone entirely.
    pub fn is_ignored(&self, meta: &WindowMeta) -> bool {
        self.ignore.iter().any(|m| m.matches(meta))
    }

    /// The structural gutter converted to pixels on the configured screen.
    pub fn gutter_px(&self) -> f32 {
        self.gutter_in * self.screen.px_per_inch()
    }

    /// True when holding the foreground still promotes (`dwell_ms > 0`).
    /// Zero is tab-only mode: the adapter never starts the timer, and
    /// the engine ignores a `Dwelled` event should one arrive anyway.
    pub fn dwell_enabled(&self) -> bool {
        self.dwell_ms > 0
    }

    /// Tab depth (border width) in pixels, clamped to half the gutter so
    /// the target stays inside its own window's half of the channel.
    pub fn tab_px(&self) -> f32 {
        (self.tab_in * self.screen.px_per_inch()).min(self.gutter_px() / 2.0)
    }

    /// Tab length in pixels (before clamping to the edge it sits on).
    pub fn tab_length_px(&self) -> f32 {
        self.tab_length_in * self.screen.px_per_inch()
    }

    /// Slim frame width in pixels, clamped to half the gutter so a frame
    /// stays in its own window's half of the channel.
    pub fn frame_px(&self) -> f32 {
        (self.frame_in * self.screen.px_per_inch()).min(self.gutter_px() / 2.0)
    }

    /// The stage frame's width in pixels, clamped to half the gutter.
    pub fn stage_px(&self) -> f32 {
        (self.stage_in * self.screen.px_per_inch()).min(self.gutter_px() / 2.0)
    }
}

/// Parse the config text format: `key = value` lines, with `[app]` and
/// `[wire]` sections repeated as needed. `#` starts a comment. Unknown
/// keys are reported rather than ignored, so typos surface immediately.
///
/// ```text
/// gutter_in = 1.5
/// dwell_ms  = 1200
///
/// [app]
/// process   = *terminal*
/// home      = left-bottom
/// focal_fit = 0.55 x 1.0
///
/// [wire]
/// a_process = claude*
/// b_title   = *claude*
/// ```
pub fn parse(text: &str) -> Result<Config, String> {
    /// Sections the parser can be inside of.
    enum Sec {
        Root,
        /// An `[app]` in progress, and the first problem found in it. A
        /// broken rule is skipped with a warning, never a parse failure
        /// (REQUESTS-2026-09-04 §8): one typo must not cost every rule.
        App(AppRule, Option<String>),
        Wire(Option<Matcher>, Option<Matcher>),
        Ignore,
    }

    /// Finish the section in progress, pushing it onto the config.
    fn flush(cfg: &mut Config, sec: Sec) -> Result<(), String> {
        match sec {
            Sec::Root => {}
            Sec::App(rule, None) => cfg.apps.push(rule),
            Sec::App(_, Some(problem)) => {
                cfg.warnings.push(format!("{problem} — [app] rule skipped"))
            }
            Sec::Ignore => {}
            Sec::Wire(a, b) => {
                let (a, b) = (
                    a.ok_or("[wire] needs a_process or a_title")?,
                    b.ok_or("[wire] needs b_process or b_title")?,
                );
                cfg.wires.push(WireRule { a, b });
            }
        }
        Ok(())
    }

    /// Parse a number, tagging the failure with the offending key.
    fn num<T: std::str::FromStr>(k: &str, v: &str) -> Result<T, String> {
        v.trim().parse().map_err(|_| format!("{k}: bad number {v:?}"))
    }

    /// Parse a `slots =` list: slot names and group names (`corners`,
    /// `sides`, `left`, `right`, `top`, `bottom`), comma-separated, in
    /// order of preference; duplicates are dropped.
    fn slot_list(v: &str) -> Result<Vec<SlotId>, String> {
        let mut out: Vec<SlotId> = Vec::new();
        for name in v.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let group = crate::layout::slot_group(name)
                .ok_or_else(|| format!("unknown slot {name:?}"))?;
            for s in group {
                if !out.contains(&s) {
                    out.push(s);
                }
            }
        }
        if out.is_empty() {
            return Err("slots wants at least one slot or group name".into());
        }
        Ok(out)
    }

    /// Apply one `key = value` line inside an `[app]` section.
    fn app_key(rule: &mut AppRule, k: &str, v: &str) -> Result<(), String> {
        match k {
            "process" => rule.matcher = Matcher::Process(v.into()),
            "title" => rule.matcher = Matcher::Title(v.into()),
            "class" => rule.matcher = Matcher::Class(v.into()),
            "match" => rule.matcher = Matcher::Substring(v.into()),
            "home" | "slots" => rule.slots = slot_list(v)?,
            "priority" => {
                rule.priority = match v.to_lowercase().as_str() {
                    "true" | "yes" | "on" => true,
                    "false" | "no" | "off" => false,
                    other => return Err(format!("priority wants true or false, got {other:?}")),
                }
            }
            "focal_fit" => {
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| "focal_fit wants WxH, e.g. 0.55 x 1.0".to_string())?;
                rule.focal_fit = Some(Fit { w: num(k, w)?, h: num(k, h)? });
            }
            other => return Err(format!("unknown key {other:?} in [app]")),
        }
        Ok(())
    }

    let mut cfg = Config::default();
    let mut sec = Sec::Root;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let where_ = |m: String| format!("line {}: {m}", n + 1);
        if line.starts_with('[') {
            flush(&mut cfg, std::mem::replace(&mut sec, Sec::Root)).map_err(where_)?;
            sec = match line {
                "[app]" => Sec::App(
                    AppRule {
                        matcher: Matcher::Any,
                        slots: Vec::new(),
                        priority: false,
                        focal_fit: None,
                    },
                    None,
                ),
                "[wire]" => Sec::Wire(None, None),
                "[ignore]" => Sec::Ignore,
                other => return Err(where_(format!("unknown section {other}"))),
            };
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| where_(format!("expected key = value, got {line:?}")))?;
        let (k, v) = (k.trim(), v.trim());
        if let Sec::App(rule, problem) = &mut sec {
            // Inside a rule, a problem marks the rule broken and parsing
            // goes on; the flush turns it into a warning.
            if let Err(e) = app_key(rule, k, v) {
                if problem.is_none() {
                    *problem = Some(where_(e));
                }
            }
            continue;
        }
        match (&mut sec, k) {
            (Sec::Root, "gutter_in") => cfg.gutter_in = num(k, v).map_err(where_)?,
            (Sec::Root, "focal_frac") => cfg.focal_frac = num(k, v).map_err(where_)?,
            (Sec::Root, "band_frac") => cfg.band_frac = num(k, v).map_err(where_)?,
            (Sec::Root, "dwell_ms") => cfg.dwell_ms = num(k, v).map_err(where_)?,
            (Sec::Root, "tap_style") => {
                cfg.tap_style = TapStyle::from_name(v)
                    .ok_or_else(|| where_(format!("tap_style wants tab or border, got {v:?}")))?
            }
            (Sec::Root, "tab_in") => cfg.tab_in = num(k, v).map_err(where_)?,
            (Sec::Root, "tab_length_in") => cfg.tab_length_in = num(k, v).map_err(where_)?,
            (Sec::Root, "frame_in") => cfg.frame_in = num(k, v).map_err(where_)?,
            (Sec::Root, "stage_in") => cfg.stage_in = num(k, v).map_err(where_)?,
            (Sec::Root, "screen_diagonal_in") => {
                cfg.screen_diagonal_in = num(k, v).map_err(where_)?
            }
            (Sec::Root, "force_active") => cfg.force_active = v == "true",
            (Sec::Ignore, "process") => cfg.ignore.push(Matcher::Process(v.into())),
            (Sec::Ignore, "title") => cfg.ignore.push(Matcher::Title(v.into())),
            (Sec::Wire(a, _), "a_process") => *a = Some(Matcher::Process(v.into())),
            (Sec::Wire(a, _), "a_title") => *a = Some(Matcher::Title(v.into())),
            (Sec::Wire(_, b), "b_process") => *b = Some(Matcher::Process(v.into())),
            (Sec::Wire(_, b), "b_title") => *b = Some(Matcher::Title(v.into())),
            _ => return Err(where_(format!("unknown key {k:?} here"))),
        }
    }
    flush(&mut cfg, sec)?;
    Ok(cfg)
}

/// The config shipped alongside the binary: sensible defaults plus
/// commented examples of every knob.
pub const EXAMPLE: &str = r#"# focal-desk configuration.
# Restart focal-desk after editing.

gutter_in          = 1.5    # structural gap; actively resizes windows
focal_frac         = 0.56   # width of the focal column (fraction of screen)
band_frac          = 0.22   # height of the top/bottom bands
dwell_ms           = 0      # 0 = off: only a tap on a window's tab promotes, a
                            # click into a window body never does. Set e.g. 1200
                            # to bring back hold-the-foreground-to-promote.
tap_style          = tab    # tab = one tab facing screen center; border = all
                            # the way around. Tap it to promote, drag it to move.
tab_in             = 0.75   # tab depth / border width, inches (max: half the gutter)
tab_length_in      = 5      # tab length along its edge, inches
frame_in           = 0.15   # the slim sides of a window's frame, inches
stage_in           = 0.35   # the stage's frame: slim all round, no tab, inches
screen_diagonal_in = 65     # physical size of the desk panel
force_active       = false  # true = manage windows even without the desk display

# Slots: focal, left-top, left-bottom, right-top, right-bottom,
#        top-1, top-2, bottom-1, bottom-2,
#        corner-tl, corner-tr, corner-bl, corner-br

[app]
process   = *windowsterminal*
home      = left-bottom
focal_fit = 0.55 x 1.0      # a terminal is a column, not a wall

[app]
process   = *code*
home      = left-top

[app]
title     = *Claude*
home      = right-top

# Assignable slots (2026-09-04): ordered preferences per app - the first
# free one is taken. Groups: corners, sides, left, right, top, bottom, or
# any slot name. `priority = true` claims the first preference even if it
# is taken; the occupant moves to its own next preference. `match =` is a
# substring of the exe, the class or the title. A window you dragged by
# its tab keeps its slot against any rule until it closes. A rule with a
# typo is logged and skipped; the rest of the file still loads.
# Uncomment to use:
#
# [app]
# match    = household          # the kiosk, wherever it shows up
# slots    = top
# priority = true
#
# [app]
# process  = *windowsterminal*
# slots    = corners
#
# [app]
# process  = *chrome*           # browsers to the sides
# slots    = sides
#
# [app]
# process  = *olk*              # mail (new Outlook): top or bottom
# slots    = top, bottom

# Never manage these, and hold the whole layout still while one of them
# is in front. Snipping Tool, the task switcher and the start menu are
# already covered by the built-in list; entries here are added to it.
# [ignore]
# process = *obs64*
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::slot_name;

    /// A WindowMeta from just a process name and a title.
    fn m(process: &str, title: &str) -> WindowMeta {
        WindowMeta { process: process.into(), title: title.into(), class: String::new() }
    }

    #[test]
    fn glob_basics() {
        assert!(glob_match("*claude*", "Claude — desktop"));
        assert!(glob_match("term*", "Terminal.exe"));
        assert!(glob_match("*.exe", "code.EXE"));
        assert!(!glob_match("mail*", "Terminal.exe"));
    }

    #[test]
    fn parses_example_config() {
        let cfg = parse(EXAMPLE).expect("example config must parse");
        assert_eq!(cfg.gutter_in, 1.5);
        assert_eq!(cfg.dwell_ms, 0, "the example ships tab-only");
        assert_eq!(cfg.apps.len(), 3, "the assignable-slot examples ship commented out");
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        let term = &cfg.apps[0];
        assert_eq!(term.slots, vec![crate::layout::slot_from_name("left-bottom").unwrap()]);
        assert!(!term.priority);
        assert!((term.focal_fit.unwrap().w - 0.55).abs() < 1e-6);
        assert!(term.matcher.matches(&m("WindowsTerminal.exe", "")));
    }

    #[test]
    fn config_errors_point_at_the_line() {
        let err = parse("gutter_in = 1.5\nnonsense_key = 3").unwrap_err();
        assert!(err.starts_with("line 2:"), "got {err}");
        // Inside [app] a problem is a warning that names the line, and
        // the rule is skipped rather than the whole file refused (§8).
        let cfg = parse("[app]\nhome = nowhere").unwrap();
        assert!(cfg.apps.is_empty());
        assert_eq!(cfg.warnings.len(), 1);
        assert!(cfg.warnings[0].starts_with("line 2:"), "got {}", cfg.warnings[0]);
        assert!(cfg.warnings[0].contains("unknown slot"), "got {}", cfg.warnings[0]);
    }

    #[test]
    fn snipping_tool_is_ignored_out_of_the_box() {
        let cfg = Config::default();
        assert!(cfg.is_ignored(&m("SnippingTool.exe", "Snipping Tool")));
        assert!(!cfg.is_ignored(&m("Code.exe", "editor")));
    }

    #[test]
    fn ignore_section_adds_to_the_defaults() {
        let cfg = parse("[ignore]\nprocess = *obs64*").unwrap();
        assert!(cfg.is_ignored(&m("obs64.exe", "")));
        // built-ins survive
        assert!(cfg.is_ignored(&m("SnippingTool.exe", "")));
    }

    // ---- assignable slots (2026-09-04, REQUESTS §8) ----------------------

    #[test]
    fn assignable_slot_rules_parse() {
        let cfg = parse(
            "[app]\nmatch = household\nslots = top\npriority = true\n\
             [app]\nprocess = *windowsterminal*\nslots = corners\n\
             [app]\nclass = Chrome_WidgetWin_1\nslots = sides\n\
             [app]\nprocess = *olk*\nslots = top, bottom\n\
             [app]\ntitle = *Claude*\nhome = right-top",
        )
        .unwrap();
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        assert_eq!(cfg.apps.len(), 5);
        let names =
            |i: usize| -> Vec<&str> { cfg.apps[i].slots.iter().map(|&s| slot_name(s)).collect() };
        assert_eq!(names(0), vec!["top-1", "top-2"]);
        assert!(cfg.apps[0].priority);
        assert!(cfg.apps[0].matcher.matches(&m("msedge.exe", "Household — kiosk")));
        assert!(cfg.apps[0].matcher.matches(&m("household.exe", "")));
        assert!(!cfg.apps[0].matcher.matches(&m("code.exe", "editor")));
        assert_eq!(names(1), vec!["corner-tl", "corner-tr", "corner-bl", "corner-br"]);
        assert!(!cfg.apps[1].priority, "priority is off unless asked");
        assert_eq!(names(2), vec!["left-top", "left-bottom", "right-top", "right-bottom"]);
        assert!(cfg.apps[2].matcher.matches(&WindowMeta {
            process: "chrome.exe".into(),
            title: "x".into(),
            class: "Chrome_WidgetWin_1".into(),
        }));
        assert_eq!(names(3), vec!["top-1", "top-2", "bottom-1", "bottom-2"]);
        assert_eq!(names(4), vec!["right-top"], "home = X is the one-slot spelling");
    }

    #[test]
    fn a_bad_rule_is_logged_and_skipped() {
        let cfg = parse(
            "gutter_in = 2\n\
             [app]\nprocess = *code*\nslots = left\n\
             [app]\nprocess = *mail*\nslots = attic\npriority = maybe\n\
             [app]\nprocess = *term*\nslots = corners\nfocal_fit = 0.55 x 1.0",
        )
        .unwrap();
        assert_eq!(cfg.gutter_in, 2.0, "the root config still loads");
        assert_eq!(cfg.apps.len(), 2, "the broken rule is dropped, its neighbours kept");
        assert!(cfg
            .apps
            .iter()
            .all(|r| !matches!(&r.matcher, Matcher::Process(p) if p == "*mail*")));
        assert_eq!(cfg.warnings.len(), 1, "one warning per broken rule: {:?}", cfg.warnings);
        let w = &cfg.warnings[0];
        assert!(w.contains("line 7"), "the first problem names its line: {w}");
        assert!(w.contains("unknown slot \"attic\""), "{w}");
        assert!(w.contains("skipped"), "{w}");
        // An unknown key inside [app] is the same kind of problem.
        let cfg = parse("[app]\nprocess = a\nhome = focal\ncolour = red").unwrap();
        assert!(cfg.apps.is_empty());
        assert!(cfg.warnings[0].contains("unknown key \"colour\""), "{}", cfg.warnings[0]);
        // Whereas a bad root key is still a refusal: nothing to fall back to.
        assert!(parse("gutter = 3").is_err());
    }

    #[test]
    fn wire_rules_need_both_ends() {
        assert!(parse("[wire]\na_process = a*").is_err());
        let cfg = parse("[wire]\na_process = a*\nb_title = *b").unwrap();
        assert_eq!(cfg.wires.len(), 1);
    }

    #[test]
    fn screen_from_px_recovers_physical_width() {
        // A 65" 16:9 panel is ~56.65" wide.
        let s = Screen::from_px(7680, 4320, 65.0);
        assert!((s.inches_w - 56.65).abs() < 0.1, "got {}", s.inches_w);
    }

    #[test]
    fn gutter_px_matches_ppi() {
        let cfg = Config::default();
        // 1.5in on a 56.65in / 7680px panel is ~203px.
        assert!((cfg.gutter_px() - 203.35).abs() < 1.0);
    }

    #[test]
    fn dwell_zero_means_off() {
        let mut cfg = Config::default();
        assert!(!cfg.dwell_enabled(), "the shipped default is tab-only (2026-09-03)");
        cfg.dwell_ms = 1200;
        assert!(cfg.dwell_enabled(), "any non-zero value is the fallback");
        assert!(parse("dwell_ms = 1200").unwrap().dwell_enabled());
        assert!(!parse("dwell_ms = 0").unwrap().dwell_enabled());
    }

    #[test]
    fn tap_target_keys_parse_and_reject_typos() {
        let cfg = parse("tap_style = border\ntab_in = 1\ntab_length_in = 3.5").unwrap();
        assert_eq!(cfg.tap_style, TapStyle::Border);
        assert!((cfg.tab_in - 1.0).abs() < 1e-6);
        assert!((cfg.tab_length_in - 3.5).abs() < 1e-6);
        assert_eq!(parse("tap_style = TAB").unwrap().tap_style, TapStyle::Tab);
        let err = parse("tap_style = ring").unwrap_err();
        assert!(err.contains("tab or border"), "got {err}");
        // The example config spells out every knob, so it must carry them.
        assert_eq!(parse(EXAMPLE).unwrap().tap_style, TapStyle::Tab);
    }

    #[test]
    fn frame_widths_parse_and_clamp() {
        let cfg = parse("frame_in = 0.2\nstage_in = 0.5").unwrap();
        assert!((cfg.frame_in - 0.2).abs() < 1e-6);
        assert!((cfg.stage_in - 0.5).abs() < 1e-6);
        let ppi = cfg.screen.px_per_inch();
        assert!((cfg.frame_px() - 0.2 * ppi).abs() < 1e-3);
        assert!((cfg.stage_px() - 0.5 * ppi).abs() < 1e-3);
        // Both stay in the window's own half of the gutter.
        let wide = parse("frame_in = 3\nstage_in = 4").unwrap();
        assert!((wide.frame_px() - wide.gutter_px() / 2.0).abs() < 1e-3, "frame clamped");
        assert!((wide.stage_px() - wide.gutter_px() / 2.0).abs() < 1e-3, "stage clamped");
        // The example carries both knobs.
        let ex = parse(EXAMPLE).unwrap();
        assert!((ex.frame_in - 0.15).abs() < 1e-6);
        assert!((ex.stage_in - 0.35).abs() < 1e-6);
    }

    #[test]
    fn tab_depth_never_exceeds_half_the_gutter() {
        let mut cfg = Config::default();
        // 0.75in of a 1.5in gutter is exactly the half — allowed.
        assert!((cfg.tab_px() - cfg.gutter_px() / 2.0).abs() < 0.5);
        cfg.tab_in = 2.0;
        assert!((cfg.tab_px() - cfg.gutter_px() / 2.0).abs() < 1e-3, "clamped");
        cfg.tab_in = 0.25;
        assert!((cfg.tab_px() - 0.25 * cfg.screen.px_per_inch()).abs() < 1e-3);
    }
}
