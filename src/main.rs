//! audio-stream-control — choose which sounds are copied into a target sink
//! (e.g. a sink whose monitor you stream to friends).
//!
//! You keep hearing everything as before. Nothing is permanent: the links belong
//! to this process, so PipeWire removes them when you untick, quit, or the app dies.
//!
//! Threads:
//!   main      – waits for "open window" requests and runs the window (egui)
//!   pipewire  – watches the audio graph and creates/removes links (pw.rs)
//!   tray      – the tray icon and its menu (ksni runs it on its own thread)

mod config;
mod icon;
mod pw;
mod tray;
mod window;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use ksni::blocking::TrayMethods;

pub const APP_ID: &str = env!("CARGO_PKG_NAME");
pub const HEADING: &str = "Sound sources:";

/// Requests to the PipeWire thread (from tray and window).
pub enum Cmd {
    Toggle(String),
    StopAll,
    Quit,
}

/// Requests to the main (window) thread.
pub enum UiMsg {
    Open,
    Quit,
}

/// One shareable source, as shown in the menu and the window.
#[derive(Clone, Debug)]
pub struct Entry {
    pub key: String,
    pub shared: bool,
    /// false = shared but not playing right now (re-linked when it comes back)
    pub present: bool,
}

/// State the PipeWire thread publishes for the window.
#[derive(Default)]
pub struct Shared {
    pub entries: Mutex<Vec<Entry>>,
    /// problem to show instead of working normally (no target, target missing)
    pub status: Mutex<Option<String>>,
    /// set while the window is open, so new state can wake it up
    pub egui_ctx: Mutex<Option<eframe::egui::Context>>,
}

const HELP: &str = "\
audio-stream-control — copy chosen sounds into a target sink

USAGE:
    audio-stream-control [--target SINK]
    audio-stream-control --install-desktop-entry

OPTIONS:
    --target SINK              use SINK instead of `target` from the config
    --install-desktop-entry    install icon + .desktop file (window icon on Wayland,
                               app menu entry), then exit
    -h, --help                 show this help

CONFIG:
    ~/.config/audio-stream-control/config.toml (created on first start)";

fn main() -> anyhow::Result<()> {
    // arguments first, so --help works even with a broken config
    let mut target_override = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--target" => {
                target_override = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--target needs a sink name"))?,
                )
            }
            "--install-desktop-entry" => return install_desktop_entry(),
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            other => anyhow::bail!("unknown argument `{other}` (see --help)"),
        }
    }

    let mut cfg = config::load()?;
    if target_override.is_some() {
        cfg.target = target_override;
    }

    let shared = Arc::new(Shared::default());
    let (cmd_tx, cmd_rx) = pipewire::channel::channel::<Cmd>();
    let (ui_tx, ui_rx) = mpsc::channel::<UiMsg>();

    let tray = tray::AppTray::new(cmd_tx.clone(), ui_tx.clone())
        .spawn()
        .map_err(|e| anyhow::anyhow!("could not create tray icon (is a tray available?): {e}"))?;

    let pw_thread = {
        let tray = tray.clone();
        let shared = shared.clone();
        let ui_tx = ui_tx.clone();
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            if let Err(e) = pw::run(cfg, cmd_rx, tray, shared) {
                eprintln!("pipewire: {e}");
            }
            let _ = ui_tx.send(UiMsg::Quit); // PipeWire part ended → whole app ends
        })
    };

    // Ctrl+C / SIGTERM → quit the normal way, so links are removed cleanly
    // (pipewire-rs only allows its own signal handling on the main thread,
    //  which is busy with the window here)
    {
        let ui_tx = ui_tx.clone();
        let shared = shared.clone();
        ctrlc::set_handler(move || {
            let _ = ui_tx.send(UiMsg::Quit);
            if let Some(ctx) = shared.egui_ctx.lock().unwrap().as_ref() {
                ctx.request_repaint(); // let an open window see the Quit
            }
        })?;
    }

    // main thread: open the window whenever asked, until told to quit
    let ui_rx = Arc::new(Mutex::new(ui_rx));
    let quitting = Arc::new(AtomicBool::new(false));
    loop {
        let msg = ui_rx.lock().unwrap().recv();
        match msg {
            Ok(UiMsg::Open) => {
                window::open(&shared, &cmd_tx, &ui_rx, &quitting, cfg.ui_scale);
                if quitting.load(Ordering::SeqCst) {
                    break;
                }
            }
            Ok(UiMsg::Quit) | Err(_) => break,
        }
    }

    // clean up: PipeWire thread drops our links, tray disappears
    let _ = cmd_tx.send(Cmd::Quit);
    let _ = pw_thread.join();
    tray.shutdown().wait();
    Ok(())
}

/// Icon + .desktop file in ~/.local/share. On Wayland the window icon comes from
/// the .desktop file whose name matches the window's app id.
fn install_desktop_entry() -> anyhow::Result<()> {
    let pngs = icon::install(APP_ID)?;
    for f in &pngs {
        println!("wrote {}", f.display());
    }
    // Full path to the largest PNG: works even when the desktop's icon-theme
    // lookup or a stale icon cache doesn't pick up ~/.local/share/icons.
    let icon_path = pngs.last().expect("at least one icon size");
    let exe = std::env::current_exe()?;
    let dir = icon::data_home().join("applications");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{APP_ID}.desktop"));
    std::fs::write(
        &file,
        format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Audio Stream Control\n\
             Comment=Choose which sounds are copied into your streaming sink\n\
             Exec={}\n\
             Icon={}\n\
             StartupWMClass={APP_ID}\n\
             Categories=AudioVideo;Audio;\n\
             Terminal=false\n",
            exe.display(),
            icon_path.display()
        ),
    )?;
    println!("wrote {}", file.display());
    println!("If the old icon is still shown, restart the app (or log out and in once).");
    Ok(())
}
