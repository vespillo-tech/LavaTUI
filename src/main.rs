//! LavaTUI — a terminal lava lamp.

mod app;
mod cli;
mod clock;
mod config;
mod light;
mod render;
mod silhouette;
mod sim;
mod theme;
mod timing;
mod ui;

use std::io;

use clap::Parser;

fn main() -> io::Result<()> {
    let mut cli = cli::Cli::parse();
    let panic_after = cli.panic_after.take();
    let session = cli.into_session();

    // `try_init` enters raw mode + the alternate screen and installs a panic
    // hook that restores the terminal; `restore` undoes it on normal exit.
    let mut terminal = ratatui::try_init().map_err(|err| {
        ratatui::restore();
        io::Error::new(
            err.kind(),
            format!("lavatui needs an interactive terminal: {err}"),
        )
    })?;
    // Chain onto ratatui's hook: switch our extra terminal modes (focus
    // reports, mouse capture) off before it restores the screen and prints
    // the panic, so the shell never receives focus or mouse escapes.
    let restore_screen = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        app::disable_terminal_modes();
        restore_screen(info);
    }));
    let result = app::run(&mut terminal, &session, panic_after);
    ratatui::restore();
    result
}
