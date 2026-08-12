//! focal-desk — entry point.
//!
//! On Windows this runs the real service. Everywhere else it runs a
//! narrated headless demo of the same engine, which doubles as living
//! documentation: `cargo run` and read the output.

use focal_core::config::{self, Config};

/// Path of the config file that sits beside the executable.
fn config_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("focal-desk.conf")))
        .unwrap_or_else(|| std::path::PathBuf::from("focal-desk.conf"))
}

/// Load the config beside the executable, writing a commented example
/// on first run so there is always something to edit.
fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => match config::parse(&text) {
            Ok(cfg) => {
                println!("config: {}", path.display());
                cfg
            }
            Err(err) => {
                eprintln!("config error in {}: {err}", path.display());
                eprintln!("falling back to defaults");
                Config::default()
            }
        },
        Err(_) => {
            let _ = std::fs::write(&path, config::EXAMPLE);
            println!("wrote starter config: {}", path.display());
            config::parse(config::EXAMPLE).unwrap_or_default()
        }
    }
}

/// Start the Windows service.
#[cfg(windows)]
fn main() {
    let cfg = load_config();
    if let Err(err) = focal_win::adapter::run(cfg) {
        eprintln!("focal-desk stopped: {err}");
    }
}

/// Print a narrated simulation of the engine on non-Windows hosts.
#[cfg(not(windows))]
fn main() {
    use focal_core::engine::{Command, Engine, Event};
    use focal_core::layout;

    println!("focal-desk — headless engine demo");
    println!("(the Windows adapter drives this exact engine with real events)\n");

    let mut engine = Engine::new(load_config());
    let script: Vec<(&str, Event)> = vec![
        ("dock into desk mode", Event::DeskMode(true)),
        (
            "editor opens",
            Event::Opened(
                0xE1,
                focal_core::config::WindowMeta {
                    process: "Code.exe".into(),
                    title: "editor".into(),
                },
            ),
        ),
        (
            "terminal opens",
            Event::Opened(
                0x7E,
                focal_core::config::WindowMeta {
                    process: "WindowsTerminal.exe".into(),
                    title: "wt".into(),
                },
            ),
        ),
        ("terminal holds foreground past dwell", Event::Promoted(0x7E)),
        ("editor holds foreground past dwell", Event::Promoted(0xE1)),
        ("click the desktop", Event::ClearStage),
        ("undock", Event::DeskMode(false)),
    ];

    for (what, event) in script {
        println!("event: {what}");
        for command in engine.handle(event) {
            match command {
                Command::Place { win, to, .. } => {
                    let home = engine.home_of(win).map(layout::slot_name).unwrap_or("?");
                    println!(
                        "   place {win:#04x} -> ({:>5.0},{:>5.0}) {:>5.0} x {:>4.0}   (home: {home})",
                        to.x, to.y, to.w, to.h
                    );
                }
                Command::Release(win) => println!("   release {win:#04x} (unmanaged)"),
            }
        }
    }
}
