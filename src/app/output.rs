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
    use ratatui::{TerminalOptions, Viewport, layout::Rect};

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
            CrosstermBackend::new(out),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 4, 2)),
            },
        )
        .unwrap();
        terminal.backend_mut().writer_mut().begin_frame().unwrap();
        terminal.resize(Rect::new(0, 0, 5, 2)).unwrap();
        terminal
            .draw(|frame| frame.render_widget("hello", frame.area()))
            .unwrap();
        assert!(terminal.backend().writer().writer.is_empty());
        let stats = terminal.backend_mut().writer_mut().finish_frame().unwrap();
        let bytes = &terminal.backend().writer().writer;
        assert!(bytes.starts_with(b"\x1b[?2026h"));
        assert!(bytes.ends_with(b"\x1b[?2026l"));
        assert_eq!(stats.writes, 1);
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
