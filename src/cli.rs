//! Command-line flags. Parsed once in `main`, then turned into a
//! [`Session`]: overrides for this run only, never written to the config.

use std::path::PathBuf;

use clap::Parser;

use crate::config::{ColorChoice, Session};

/// A terminal lava lamp.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Just the lamp: no panels, status bar or overlays.
    #[arg(short, long)]
    pub minimal: bool,

    /// Target render frames per second (the simulation rate is fixed separately).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=240))]
    pub fps: Option<u32>,

    /// Render style for this session (e.g. solid, outline, heatmap, ascii, dither).
    #[arg(long, value_name = "NAME")]
    pub style: Option<String>,

    /// Palette for this session (lava, ultraviolet, abyss, toxic, synthwave, mono, paper, ansi).
    #[arg(long, value_name = "NAME")]
    pub palette: Option<String>,

    /// Colour depth, instead of detecting it from the environment.
    #[arg(long, value_name = "DEPTH")]
    pub color: Option<ColorChoice>,

    /// Seed the wax simulation: the same seed always plays out the same lamp
    /// (default: a new seed every launch).
    #[arg(long, value_name = "U64")]
    pub seed: Option<u64>,

    /// Read and write settings here instead of the XDG config dir.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Exit after rendering N frames (smoke tests / benchmarking).
    #[arg(long, value_name = "N", hide = true)]
    pub frames: Option<u64>,
}

impl Cli {
    pub fn into_session(self) -> Session {
        Session {
            minimal: self.minimal,
            fps: self.fps,
            style: self.style,
            palette: self.palette,
            color: self.color,
            seed: self.seed,
            max_frames: self.frames,
            config_path: self.config,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_no_flags() {
        let session = Cli::parse_from(["lavatui"]).into_session();
        assert_eq!(session, Session::default());
    }

    #[test]
    fn parses_minimal_and_fps() {
        let session = Cli::parse_from(["lavatui", "--minimal", "--fps", "30"]).into_session();
        assert!(session.minimal);
        assert_eq!(session.fps, Some(30));
        assert!(Cli::parse_from(["lavatui", "-m"]).into_session().minimal);
    }

    #[test]
    fn parses_seed() {
        let session = Cli::parse_from(["lavatui", "--seed", "18446744073709551615"]).into_session();
        assert_eq!(session.seed, Some(u64::MAX));
        assert!(Cli::try_parse_from(["lavatui", "--seed", "-1"]).is_err());
    }

    #[test]
    fn parses_color_style_palette_config() {
        let session = Cli::parse_from([
            "lavatui",
            "--color",
            "256",
            "--style",
            "ascii",
            "--palette",
            "mono",
            "--config",
            "/tmp/x.toml",
        ])
        .into_session();
        assert_eq!(session.color, Some(ColorChoice::Ansi256));
        assert_eq!(session.style.as_deref(), Some("ascii"));
        assert_eq!(session.palette.as_deref(), Some("mono"));
        assert_eq!(session.config_path, Some(PathBuf::from("/tmp/x.toml")));
        assert!(Cli::try_parse_from(["lavatui", "--color", "lots"]).is_err());
        for depth in ["auto", "truecolor", "16", "none"] {
            assert!(Cli::try_parse_from(["lavatui", "--color", depth]).is_ok());
        }
    }

    #[test]
    fn rejects_out_of_range_fps() {
        assert!(Cli::try_parse_from(["lavatui", "--fps", "0"]).is_err());
        assert!(Cli::try_parse_from(["lavatui", "--fps", "1000"]).is_err());
    }
}
