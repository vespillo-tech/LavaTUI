//! LavaTUI — a terminal lava lamp.

mod app;
mod cells;
mod cli;
mod clock;
mod config;
mod demo;
mod diag;
mod disk_cache;
mod dock;
mod glyphs;
mod graphics;
mod lyrics;
mod media;
mod render;
mod sim;
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

#[cfg(test)]
mod tests {
    /// The project part of the agent instructions is one text in two
    /// files (each tool reads its own); the issue-tracker notes above it
    /// are generated per tool and may differ.
    #[test]
    fn claude_md_and_agents_md_share_the_project_section() {
        let project = |file: &'static str| {
            let at = file.find("## Project: LavaTUI").expect("project section");
            file[at..].replace("\r\n", "\n")
        };
        assert!(
            project(include_str!("../CLAUDE.md")) == project(include_str!("../AGENTS.md")),
            "CLAUDE.md and AGENTS.md differ from `## Project: LavaTUI` on: copy one over the other"
        );
    }

    /// tools/ghostty_native.py runs helpers in LavaTUI's own terminal:
    /// one that prints (screencapture failing to save on a full disk) puts
    /// text under the lamp, so each sends its output elsewhere.
    #[test]
    fn the_native_harness_keeps_its_helpers_off_the_screen() {
        let tool = include_str!("../tools/ghostty_native.py");
        let start = tool.find("WRAPPER = ").expect("the wrapper script");
        let end = start + tool[start..][12..].find("\"\"\"").expect("its end") + 12;
        for line in tool[start..end].lines() {
            for helper in ["screencapture", "stty"] {
                if line.contains(helper) && !line.trim_start().starts_with('#') {
                    assert!(line.contains("2>"), "{helper} may print on screen: {line}");
                }
            }
        }
    }
}
