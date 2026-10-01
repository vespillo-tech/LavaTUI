//! Opt-in frame trace: rows stay in memory until exit, so tracing itself
//! performs no file I/O on the frame path. Times are microseconds.
//! `diff_us` is residual terminal work (diff/encoding, size check, buffer
//! swap, local clock and bookkeeping), excluding measured tick/draw/I/O.
//! `writes` counts successful calls to the underlying Write implementation,
//! not OS syscalls. `deadline_miss_us` is lateness at frame start.
use super::output::OutputStats;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Frame {
    pub frame: u64,
    pub wait_start: Instant,
    pub wait_end: Instant,
    pub wait_cpu_us: u64,
    pub started: Instant,
    pub drawn: Instant,
    pub deadline: Instant,
    pub interval: Duration,
    pub input: bool,
    pub tick_us: u64,
    pub draw_us: u64,
    pub total_us: u64,
    pub sim_steps: u32,
    pub sim_dt: f64,
    pub sim_feed_s: f64,
    pub save_us: u64,
    pub output: OutputStats,
    pub fps: u32,
}

pub struct Trace {
    file: Option<BufWriter<File>>,
    origin: Instant,
    frames: Vec<Frame>,
}

impl Trace {
    pub fn new(path: Option<&Path>) -> io::Result<Self> {
        Ok(Self {
            file: path.map(File::create).transpose()?.map(BufWriter::new),
            origin: Instant::now(),
            frames: Vec::new(),
        })
    }

    pub fn record(&mut self, frame: Frame) {
        if self.file.is_some() {
            self.frames.push(frame);
        }
    }

    pub fn enabled(&self) -> bool {
        self.file.is_some()
    }

    pub fn finish(&mut self) -> io::Result<()> {
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        writeln!(
            file,
            "frame,wait_start_us,wait_end_us,start_us,end_us,interval_us,input,tick_us,draw_us,diff_us,write_us,flush_us,bytes,writes,sim_steps,sim_dt_s,sim_feed_s,save_us,deadline_miss_us,fps,wait_cpu_us"
        )?;
        let us = |at: Instant| at.saturating_duration_since(self.origin).as_micros();
        for f in self.frames.drain(..) {
            writeln!(
                file,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.9},{:.9},{},{},{},{}",
                f.frame,
                us(f.wait_start),
                us(f.wait_end),
                us(f.started),
                us(f.drawn),
                f.interval.as_micros(),
                u8::from(f.input),
                f.tick_us,
                f.draw_us,
                f.total_us
                    .saturating_sub(f.tick_us + f.draw_us + f.output.write_us + f.output.flush_us),
                f.output.write_us,
                f.output.flush_us,
                f.output.bytes,
                f.output.writes,
                f.sim_steps,
                f.sim_dt,
                f.sim_feed_s,
                f.save_us,
                f.started.saturating_duration_since(f.deadline).as_micros(),
                f.fps,
                f.wait_cpu_us
            )?;
        }
        file.flush()
    }
}
