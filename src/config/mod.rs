//! Runtime settings. Currently built from CLI flags only; TOML load/save
//! (XDG config dir) lands here later.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// "Just the lamp" mode: hide all chrome.
    pub minimal: bool,
    /// Target render frame rate.
    pub fps: u32,
    /// Quit after this many rendered frames (`None` = run until the user quits).
    pub max_frames: Option<u64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            minimal: false,
            fps: 60,
            max_frames: None,
        }
    }
}
