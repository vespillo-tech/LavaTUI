//! LavaTUI — a terminal lava lamp.

mod app;
mod cells;
mod cli;
mod clock;
mod config;
mod demo;
mod disk_cache;
mod dock;
mod glyphs;
mod graphics;
mod lyrics;
// Partly used so far: the music widget (lava-75z.2) reads it; play_uri&co are
// for the library UI (lava-75z.5).
#[allow(dead_code, unused_imports)]
mod media;
mod render;
mod sim;
// Not wired into the UI yet (lava-75z.5 does that).
#[allow(dead_code)]
mod spotify_web;
mod theme;
mod thread_qos;
mod timing;
mod ui;

use std::io::{self, IsTerminal};
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let mut cli = cli::Cli::parse();
    let panic_after = cli.panic_after.take();
    let trace = cli
        .trace
        .take()
        .or_else(|| std::env::var_os("LAVATUI_TRACE").map(Into::into));
    let session = cli.into_session();

    // Both ends must be the terminal: without stdin there are no keys, and
    // with stdout redirected the frames would land in a file.
    let not_a_tty = [
        ("stdin", io::stdin().is_terminal()),
        ("stdout", io::stdout().is_terminal()),
    ]
    .into_iter()
    .find(|(_, tty)| !tty);
    if let Some((name, _)) = not_a_tty {
        return fail(&format!(
            "needs an interactive terminal ({name} is not a terminal)"
        ));
    }

    // `try_init` enters raw mode + the alternate screen and installs a panic
    // hook that restores the terminal; `restore` undoes it on normal exit.
    let mut terminal = match ratatui::try_init().and_then(|_| app::new_terminal(trace.is_some())) {
        Ok(terminal) => terminal,
        Err(err) => {
            ratatui::restore();
            return fail(&format!("needs an interactive terminal ({err})"));
        }
    };
    // Chain onto ratatui's hook: switch our extra terminal modes (focus
    // reports, mouse capture) off before it restores the screen and prints
    // the panic, so the shell never receives focus or mouse escapes.
    let restore_screen = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        app::disable_terminal_modes();
        restore_screen(info);
    }));
    let result = app::run(&mut terminal, &session, panic_after, trace.as_deref());
    ratatui::restore();
    match result {
        Ok(report) => {
            for line in report {
                eprintln!("lavatui: {line}");
            }
            ExitCode::SUCCESS
        }
        Err(err) => fail(&err.to_string()),
    }
}

/// One line on stderr, exit 1: no Debug dump of the error.
fn fail(message: &str) -> ExitCode {
    eprintln!("lavatui: {message}");
    ExitCode::FAILURE
}
