//! LavaTUI — a terminal lava lamp.

mod app;
mod cli;
mod clock;
mod config;
mod dock;
// Not wired into the app yet: the now-playing widget (lava-75z.2) will be.
#[allow(dead_code, unused_imports)]
mod media;
mod render;
mod sim;
mod theme;
mod timing;
mod ui;

use std::io::{self, IsTerminal};
use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let mut cli = cli::Cli::parse();
    let panic_after = cli.panic_after.take();
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
    let mut terminal = match ratatui::try_init() {
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
    let result = app::run(&mut terminal, &session, panic_after);
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
