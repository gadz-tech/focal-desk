//! focal-desk binary.
//!
//! On Windows (with focal-win's `win32` feature enabled) this will run
//! the real adapter. Everywhere else it runs a narrated headless demo
//! of the engine — the same state machine the adapter will drive —
//! which doubles as living documentation: run `cargo run` and read.

use focal_core::config::{AppRule, Config, Fit, Matcher, WindowMeta};
use focal_core::engine::{Command, Engine, Event};
use focal_core::layout;

fn main() {
    println!("focal-desk — headless engine demo");
    println!("(the Windows adapter drives this exact engine with real events)\n");

    let mut cfg = Config::default();
    cfg.apps = vec![AppRule {
        matcher: Matcher::Process("*terminal*".into()),
        home: None,
        focal_fit: Some(Fit { w: 0.55, h: 1.0 }),
    }];
    let mut engine = Engine::new(cfg);

    let script: Vec<(&str, Event)> = vec![
        ("dock into desk mode", Event::DeskMode(true)),
        (
            "editor opens",
            Event::Opened(0xE1, WindowMeta { process: "code.exe".into(), title: "editor".into() }),
        ),
        (
            "terminal opens",
            Event::Opened(
                0x7E,
                WindowMeta { process: "windowsterminal.exe".into(), title: "wt".into() },
            ),
        ),
        ("terminal holds foreground past dwell", Event::Promoted(0x7E)),
        ("editor holds foreground past dwell", Event::Promoted(0xE1)),
        ("undock", Event::DeskMode(false)),
    ];

    for (what, ev) in script {
        println!("event: {what}");
        for c in engine.handle(ev) {
            match c {
                Command::Place { win, to, .. } => {
                    let slot = engine
                        .home_of(win)
                        .map(layout::slot_name)
                        .unwrap_or("?");
                    println!(
                        "   place {win:#04x} -> ({:>5.0},{:>5.0}) {:>5.0} x {:>4.0}   (home: {slot})",
                        to.x, to.y, to.w, to.h
                    );
                }
                Command::Release(w) => println!("   release {w:#04x} (unmanaged)"),
            }
        }
    }
}
