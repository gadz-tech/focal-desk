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

/// What the adapter reports about a window when it appears.
#[derive(Clone, Debug)]
pub struct WindowMeta {
    pub process: String,
    pub title: String,
}

/// Matches windows to rules. Globs are case-insensitive, `*` wildcard.
#[derive(Clone, Debug)]
pub enum Matcher {
    Process(String),
    Title(String),
    Any,
}

impl Matcher {
    /// True when this matcher's glob matches the relevant window field.
    pub fn matches(&self, meta: &WindowMeta) -> bool {
        match self {
            Matcher::Process(g) => glob_match(g, &meta.process),
            Matcher::Title(g) => glob_match(g, &meta.title),
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
    /// Preferred home slot; `None` means "first free by priority".
    pub home: Option<SlotId>,
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
    /// How long a window must hold the foreground before promotion.
    pub dwell_ms: u64,
    /// Physical diagonal of the desk panel, used to convert the
    /// inch-based gutter into pixels once the mode is known.
    pub screen_diagonal_in: f32,
    /// Manage windows even when the desk display isn't detected.
    /// Handy for trying the layout on a laptop screen.
    pub force_active: bool,
    pub apps: Vec<AppRule>,
    pub wires: Vec<WireRule>,
}

impl Default for Config {
    /// The setup this project is built around: 65" 8K, 1.5" gutter,
    /// 56% focal column, 22% bands, 1.2s dwell.
    fn default() -> Self {
        Self {
            screen: Screen::desk_65_8k(),
            gutter_in: 1.5,
            focal_frac: 0.56,
            band_frac: 0.22,
            dwell_ms: 1200,
            screen_diagonal_in: 65.0,
            force_active: false,
            apps: Vec::new(),
            wires: Vec::new(),
        }
    }
}

impl Config {
    /// The structural gutter converted to pixels on the configured screen.
    pub fn gutter_px(&self) -> f32 {
        self.gutter_in * self.screen.px_per_inch()
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
        App(AppRule),
        Wire(Option<Matcher>, Option<Matcher>),
    }

    /// Finish the section in progress, pushing it onto the config.
    fn flush(cfg: &mut Config, sec: Sec) -> Result<(), String> {
        match sec {
            Sec::Root => {}
            Sec::App(rule) => cfg.apps.push(rule),
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
                "[app]" => Sec::App(AppRule { matcher: Matcher::Any, home: None, focal_fit: None }),
                "[wire]" => Sec::Wire(None, None),
                other => return Err(where_(format!("unknown section {other}"))),
            };
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| where_(format!("expected key = value, got {line:?}")))?;
        let (k, v) = (k.trim(), v.trim());
        match (&mut sec, k) {
            (Sec::Root, "gutter_in") => cfg.gutter_in = num(k, v).map_err(where_)?,
            (Sec::Root, "focal_frac") => cfg.focal_frac = num(k, v).map_err(where_)?,
            (Sec::Root, "band_frac") => cfg.band_frac = num(k, v).map_err(where_)?,
            (Sec::Root, "dwell_ms") => cfg.dwell_ms = num(k, v).map_err(where_)?,
            (Sec::Root, "screen_diagonal_in") => {
                cfg.screen_diagonal_in = num(k, v).map_err(where_)?
            }
            (Sec::Root, "force_active") => cfg.force_active = v == "true",
            (Sec::App(rule), "process") => rule.matcher = Matcher::Process(v.into()),
            (Sec::App(rule), "title") => rule.matcher = Matcher::Title(v.into()),
            (Sec::App(rule), "home") => {
                rule.home = Some(
                    crate::layout::slot_from_name(v)
                        .ok_or_else(|| where_(format!("unknown slot {v:?}")))?,
                )
            }
            (Sec::App(rule), "focal_fit") => {
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| where_("focal_fit wants WxH, e.g. 0.55 x 1.0".into()))?;
                rule.focal_fit = Some(Fit {
                    w: num(k, w).map_err(where_)?,
                    h: num(k, h).map_err(where_)?,
                });
            }
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
dwell_ms           = 1200   # how long a window must hold focus to be promoted
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
"#;

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(cfg.dwell_ms, 1200);
        assert_eq!(cfg.apps.len(), 3);
        let term = &cfg.apps[0];
        assert_eq!(term.home, crate::layout::slot_from_name("left-bottom"));
        assert!((term.focal_fit.unwrap().w - 0.55).abs() < 1e-6);
        assert!(term.matcher.matches(&WindowMeta {
            process: "WindowsTerminal.exe".into(),
            title: String::new(),
        }));
    }

    #[test]
    fn config_errors_point_at_the_line() {
        let err = parse("gutter_in = 1.5\nnonsense_key = 3").unwrap_err();
        assert!(err.starts_with("line 2:"), "got {err}");
        let err = parse("[app]\nhome = nowhere").unwrap_err();
        assert!(err.contains("unknown slot"), "got {err}");
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
}
