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
    pub fn px_per_inch(&self) -> f32 {
        self.px_w as f32 / self.inches_w
    }
    /// The target display: 65" 16:9 at 8K (~135.6 ppi).
    pub fn desk_65_8k() -> Self {
        Self { px_w: 7680, px_h: 4320, inches_w: 56.65 }
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
    pub apps: Vec<AppRule>,
    pub wires: Vec<WireRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            screen: Screen::desk_65_8k(),
            gutter_in: 1.5,
            focal_frac: 0.56,
            band_frac: 0.22,
            dwell_ms: 1200,
            apps: Vec::new(),
            wires: Vec::new(),
        }
    }
}

impl Config {
    pub fn gutter_px(&self) -> f32 {
        self.gutter_in * self.screen.px_per_inch()
    }
}

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
    fn gutter_px_matches_ppi() {
        let cfg = Config::default();
        // 1.5in on a 56.65in / 7680px panel is ~203px.
        assert!((cfg.gutter_px() - 203.35).abs() < 1.0);
    }
}
