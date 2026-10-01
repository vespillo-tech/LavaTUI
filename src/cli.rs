//! Command-line flags. Parsed once in `main`, then folded into [`Config`].

use clap::Parser;

use crate::config::Config;

/// A terminal lava lamp.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Just the lamp: no panels, status bar or overlays.
    #[arg(long)]
    pub minimal: bool,

    /// Target render frames per second (the simulation rate is fixed separately).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..=240))]
    pub fps: Option<u32>,

    /// Seed the wax simulation: the same seed always plays out the same lamp
    /// (default: a new seed every launch).
    #[arg(long, value_name = "U64")]
    pub seed: Option<u64>,

    /// Exit after rendering N frames (smoke tests / benchmarking).
    #[arg(long, value_name = "N", hide = true)]
    pub frames: Option<u64>,
}

impl Cli {
    pub fn into_config(self) -> Config {
        let defaults = Config::default();
        Config {
            minimal: self.minimal,
            fps: self.fps.unwrap_or(defaults.fps),
            max_frames: self.frames,
            seed: self.seed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_no_flags() {
        let config = Cli::parse_from(["lavatui"]).into_config();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn parses_minimal_and_fps() {
        let config = Cli::parse_from(["lavatui", "--minimal", "--fps", "30"]).into_config();
        assert!(config.minimal);
        assert_eq!(config.fps, 30);
    }

    #[test]
    fn parses_seed() {
        let config = Cli::parse_from(["lavatui", "--seed", "18446744073709551615"]).into_config();
        assert_eq!(config.seed, Some(u64::MAX));
        assert!(Cli::try_parse_from(["lavatui", "--seed", "-1"]).is_err());
    }

    #[test]
    fn rejects_out_of_range_fps() {
        assert!(Cli::try_parse_from(["lavatui", "--fps", "0"]).is_err());
        assert!(Cli::try_parse_from(["lavatui", "--fps", "1000"]).is_err());
    }
}
