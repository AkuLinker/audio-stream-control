//! ~/.config/audio-stream-control/config.toml
//! Created with commented examples on first start. Read once at startup.

use std::path::PathBuf;

use serde::Deserialize;

const TEMPLATE: &str = r#"# audio-stream-control configuration
# Changes take effect after restarting the app.

# Sink that receives the copies. Others hear its *monitor*
# (e.g. choose "Monitor of …" as input in Discord / RustDesk).
# List sink names with:  pactl list short sinks
# target = "loopback-sink"

# Size of text and buttons in the window (1.0 = egui's small default).
ui_scale = 1.5

# Sinks to offer as one entry each, e.g. a sink your games play into.
# Everything playing into such a sink is shared together.
# [[sources]]
# sink  = "game-sink"
# label = "Game Audio"
"#;

#[derive(Deserialize, Clone, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub target: Option<String>,
    pub ui_scale: f32,
    pub sources: Vec<SinkSource>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            target: None,
            ui_scale: 1.5,
            sources: Vec::new(),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct SinkSource {
    pub sink: String,
    pub label: String,
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join(env!("CARGO_PKG_NAME")).join("config.toml")
}

/// Read the config, creating the commented template if it doesn't exist yet.
pub fn load() -> anyhow::Result<Config> {
    let path = path();
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, TEMPLATE)?;
        eprintln!("created {}", path.display());
    }
    let text = std::fs::read_to_string(&path)?;
    let mut cfg: Config =
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    // treat target = "" like no target
    cfg.target = cfg.target.filter(|t| !t.trim().is_empty());
    cfg.ui_scale = cfg.ui_scale.clamp(0.5, 4.0);
    Ok(cfg)
}
