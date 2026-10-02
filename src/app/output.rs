//! Terminal writer and measurements at the actual I/O boundary.
use ratatui::crossterm::{
    queue,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io::{self, Write};
use std::time::Instant;

pub type AppTerminal = Terminal<CrosstermBackend<Output<io::Stdout>>>;

pub fn new_terminal(measure: bool) -> io::Result<AppTerminal> {
    Terminal::new(CrosstermBackend::new(Output::new(io::stdout(), measure)))
}

// Legacy Windows consoles mix WinAPI cursor/colour operations with text
// writes; buffering only text would reorder them. Keep the original writer
// semantics there. Modern VT consoles and Unix use the frame batch.
pub(super) fn ansi_output() -> bool {
    #[cfg(windows)]
    {
        ratatui::crossterm::ansi_support::supports_ansi()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[derive(Clone, Copy, Default)]
pub struct OutputStats {
    pub bytes: usize,
    pub writes: usize,
    pub write_us: u64,
    pub flush_us: u64,
}

pub struct Output<W> {
    writer: W,
    measure: bool,
    stats: OutputStats,
    frame: bool,
    batch: bool,
    buffer: Vec<u8>,
    write_time: std::time::Duration,
}

impl<W: Write> Output<W> {
    fn new(writer: W, measure: bool) -> Self {
        Self {
            writer,
            measure,
            stats: OutputStats::default(),
            frame: false,
            batch: ansi_output(),
            buffer: Vec::with_capacity(64 * 1024),
            write_time: std::time::Duration::ZERO,
        }
    }

    pub fn begin_frame(&mut self) -> io::Result<()> {
        self.stats = OutputStats::default();
        self.write_time = std::time::Duration::ZERO;
        if !self.batch {
            return Ok(());
        }
        self.buffer.clear();
        self.frame = true;
        queue!(self, BeginSynchronizedUpdate)
    }

    pub fn finish_frame(&mut self) -> io::Result<OutputStats> {
        if self.batch {
            queue!(self, EndSynchronizedUpdate)?;
            self.frame = false;
            // Retain the allocation and let Write::write_all retry short/
            // interrupted writes. Count actual Write calls, not just batches.
            let bytes = std::mem::take(&mut self.buffer);
            let result = self.write_all(&bytes);
            self.buffer = bytes;
            result?;
        }
        self.flush()?;
        Ok(self.stats)
    }
}

impl<W: Write> Write for Output<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.frame {
            self.buffer.extend_from_slice(bytes);
            return Ok(bytes.len());
        }
        if !self.measure {
            return self.writer.write(bytes);
        }
        let at = Instant::now();
        let result = self.writer.write(bytes);
        self.write_time += at.elapsed();
        self.stats.write_us = self.write_time.as_micros() as u64;
        if let Ok(n) = result {
            self.stats.bytes += n;
            self.stats.writes += 1;
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.frame {
            return Ok(());
        }
        if !self.measure {
            return self.writer.flush();
        }
        let at = Instant::now();
        let result = self.writer.flush();
        self.stats.flush_us += at.elapsed().as_micros() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_buffered_until_its_synchronized_end() {
        let mut out = Output::new(Vec::new(), true);
        out.batch = true;
        out.begin_frame().unwrap();
        out.write_all(b"one").unwrap();
        out.flush().unwrap(); // ratatui flush must not expose a partial frame
        out.write_all(b"two").unwrap();
        assert!(out.writer.is_empty());
        let stats = out.finish_frame().unwrap();
        assert_eq!(out.writer, b"\x1b[?2026honetwo\x1b[?2026l");
        assert_eq!(stats.bytes, out.writer.len());
        assert_eq!(stats.writes, 1);
        out.begin_frame().unwrap();
        out.write_all(b"next").unwrap();
        out.finish_frame().unwrap();
        assert!(out.writer.ends_with(b"\x1b[?2026hnext\x1b[?2026l"));
    }
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    #[test]
    fn legacy_console_flushes_text_before_winapi_commands() {
        let mut out = Output::new(Vec::new(), true);
        out.batch = false;
        out.begin_frame().unwrap();
        out.write_all(b"text").unwrap();
        out.flush().unwrap();
        assert_eq!(out.writer, b"text");
        out.finish_frame().unwrap();
        assert_eq!(out.writer, b"text");
    }
}

#[cfg(test)]
mod backend_tests {
    use super::*;
    use ratatui::backend::{Backend, ClearType, WindowSize};
    use ratatui::buffer::Cell;
    use ratatui::layout::{Position, Size};
    use ratatui::{TerminalOptions, Viewport, layout::Rect};

    /// Batching must preserve every colour change over successive lamp
    /// diffs, including the fg/bg swaps used for half-block curves.
    #[test]
    fn synchronized_lamp_diffs_match_unbuffered_ansi() {
        use crate::render::{LampOptions, LampState, LampView, StyleId};
        use crate::sim::{Field, World};
        use crate::theme::{ColorDepth, Palette, Theme};
        use ratatui::buffer::Buffer;
        use ratatui::widgets::StatefulWidget;

        if !ansi_output() {
            return;
        }
        for (w, h) in [(160, 45), (200, 60)] {
            let mut world = World::new(2, f64::from(w) / (2.0 * f64::from(h)));
            world.prewarm(600, 1.0 / 120.0);
            let theme = Theme::new(&Palette::all()[0], ColorDepth::TrueColor);
            for name in ["solid", "braille", "outline", "synthwave", "chrome"] {
                let area = Rect::new(7, 3, w, h);
                let outer = Rect::new(0, 0, w + 20, h + 8);
                let (mut prev, mut next) = (Buffer::empty(outer), Buffer::empty(outer));
                let mut plain = CrosstermBackend::new(Vec::new());
                let mut out = Output::new(Vec::new(), false);
                out.batch = true;
                let mut batched = CrosstermBackend::new(out);
                let mut field = Field::default();
                let mut state = LampState::default();
                for frame in 0..12 {
                    world.step(1.0 / 120.0);
                    world.step(1.0 / 120.0);
                    field.prepare(&world, 1.0);
                    next.reset();
                    LampView {
                        field: &field,
                        style: StyleId::by_name(name).unwrap().style(),
                        theme: &theme,
                        time: f64::from(frame) / 60.0,
                        options: LampOptions::default(),
                    }
                    .render(area, &mut next, &mut state);
                    plain.writer_mut().clear();
                    batched.writer_mut().writer.clear();
                    plain.draw(prev.diff_iter(&next)).unwrap();
                    Backend::flush(&mut plain).unwrap();
                    if std::env::var_os("NO_COLOR").is_none() {
                        assert!(plain.writer().windows(5).any(|b| b == b"38;2;"));
                        assert!(plain.writer().windows(5).any(|b| b == b"48;2;"));
                    }
                    batched.writer_mut().begin_frame().unwrap();
                    batched.draw(prev.diff_iter(&next)).unwrap();
                    Backend::flush(&mut batched).unwrap();
                    batched.writer_mut().finish_frame().unwrap();
                    let bytes = &batched.writer().writer;
                    assert!(bytes.starts_with(b"\x1b[?2026h"));
                    assert!(bytes.ends_with(b"\x1b[?2026l"));
                    assert_eq!(
                        &bytes[8..bytes.len() - 8],
                        plain.writer(),
                        "{name} {w}x{h} frame {frame}",
                    );
                    std::mem::swap(&mut prev, &mut next);
                }
            }
        }
    }

    #[test]
    fn ratatui_clear_diff_and_flush_share_the_frame_batch() {
        // Encoding assertions require VT; legacy WinAPI ordering is covered
        // by the passthrough test instead.
        if !ansi_output() {
            return;
        }
        let mut out = Output::new(Vec::new(), true);
        out.batch = true;
        let mut terminal = Terminal::with_options(
            Sized(CrosstermBackend::new(out), Size::new(5, 2)),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 4, 2)),
            },
        )
        .unwrap();
        terminal.backend_mut().0.writer_mut().begin_frame().unwrap();
        terminal.resize(Rect::new(0, 0, 5, 2)).unwrap();
        terminal
            .draw(|frame| frame.render_widget("hello", frame.area()))
            .unwrap();
        assert!(terminal.backend().0.writer().writer.is_empty());
        let stats = terminal
            .backend_mut()
            .0
            .writer_mut()
            .finish_frame()
            .unwrap();
        let bytes = &terminal.backend().0.writer().writer;
        assert!(bytes.starts_with(b"\x1b[?2026h"));
        assert!(bytes.ends_with(b"\x1b[?2026l"));
        assert_eq!(stats.writes, 1);
    }

    /// A backend that says the screen is `.1`: a fixed viewport's resize
    /// asks the size, and crossterm's answer depends on where the tests
    /// run (no tty and no `$TERM` in a container: an error; a 0x0 pty).
    struct Sized<B>(B, Size);

    impl<B: Backend> Backend for Sized<B> {
        type Error = B::Error;

        fn draw<'a, I>(&mut self, content: I) -> Result<(), B::Error>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            self.0.draw(content)
        }
        fn hide_cursor(&mut self) -> Result<(), B::Error> {
            self.0.hide_cursor()
        }
        fn show_cursor(&mut self) -> Result<(), B::Error> {
            self.0.show_cursor()
        }
        fn get_cursor_position(&mut self) -> Result<Position, B::Error> {
            self.0.get_cursor_position()
        }
        fn set_cursor_position<P: Into<Position>>(&mut self, at: P) -> Result<(), B::Error> {
            self.0.set_cursor_position(at)
        }
        fn clear(&mut self) -> Result<(), B::Error> {
            self.0.clear()
        }
        fn clear_region(&mut self, clear_type: ClearType) -> Result<(), B::Error> {
            self.0.clear_region(clear_type)
        }
        fn size(&self) -> Result<Size, B::Error> {
            Ok(self.1)
        }
        fn window_size(&mut self) -> Result<WindowSize, B::Error> {
            Ok(WindowSize {
                columns_rows: self.1,
                pixels: Size::new(self.1.width * 8, self.1.height * 16),
            })
        }
        fn flush(&mut self) -> Result<(), B::Error> {
            Backend::flush(&mut self.0)
        }
    }

    #[derive(Default)]
    struct ShortWriter {
        bytes: Vec<u8>,
        interrupted: bool,
    }

    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            let n = bytes.len().min(3);
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn frame_output_retries_short_and_interrupted_writes() {
        let mut out = Output::new(ShortWriter::default(), true);
        out.batch = true;
        out.begin_frame().unwrap();
        out.write_all(b"frame").unwrap();
        let stats = out.finish_frame().unwrap();
        assert_eq!(out.writer.bytes, b"\x1b[?2026hframe\x1b[?2026l");
        assert_eq!(stats.bytes, out.writer.bytes.len());
        assert!(stats.writes > 1);
    }
}
