//! LavaTUI — a terminal lava lamp.

mod app;
mod cli;
mod clock;
mod config;
mod light;
mod render;
mod sim;
mod theme;
mod timing;
mod ui;

use std::io;

use clap::Parser;

fn main() -> io::Result<()> {
    let session = cli::Cli::parse().into_session();

    // `try_init` enters raw mode + the alternate screen and installs a panic
    // hook that restores the terminal; `restore` undoes it on normal exit.
    let mut terminal = ratatui::try_init().map_err(|err| {
        ratatui::restore();
        io::Error::new(
            err.kind(),
            format!("lavatui needs an interactive terminal: {err}"),
        )
    })?;
    let result = app::run(&mut terminal, &session);
    ratatui::restore();
    result
}
