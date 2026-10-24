// made by CasualArclamp
//! DecDAB command line.
//!
//! - `blocks`: the Band III block plan.
//! - `scan`: step the HackRF over Band III and say which blocks carry an
//!   ensemble (a flat 1.536 MHz block with a gap either side).
//! - `capture`: record a block (or a frequency) from the HackRF to an IQ
//!   file named by its tuning, so the GUI and CLI can read the centre and
//!   rate back from the name.
//! - `decode`: one service to a WAV file, with the decoding figures.
//! - `info`: lock to an ensemble (a capture or a block on the HackRF) and
//!   list what its FIC says: label, services, sub-channels, the time.

mod decode;
mod info;
mod source;

use std::path::PathBuf;
use std::time::Instant;

#[allow(unused_imports)] // `bail` is only used without the hackrf feature
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use decdab_core::channel::{BAND_III, block_at, parse_block, scan_blocks};

#[derive(Parser)]
#[command(name = "decdab", version, about = "DAB / DAB+ receiver")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the Band III blocks and their centre frequencies.
    Blocks,
    /// Step the HackRF over Band III and report each block's level and
    /// whether it looks like a DAB ensemble.
    Scan {
        #[command(flatten)]
        gains: Gains,
        /// Dwell on each block, seconds.
        #[arg(long, default_value_t = 0.3)]
        dwell: f64,
    },
    /// List an ensemble's services from a capture or a block on the HackRF.
    Info {
        /// An IQ file, or a block ("12B") / frequency in MHz for the HackRF.
        source: String,
        /// Seconds to listen.
        #[arg(long, default_value_t = 4.0)]
        seconds: f64,
        #[command(flatten)]
        gains: Gains,
    },
    /// Decode one service to a WAV file and report on it.
    Decode(Box<decode::DecodeArgs>),
    /// Record a block ("12B") or a frequency in MHz from the HackRF.
    Capture {
        /// A Band III block ("12B") or a centre frequency in MHz.
        what: String,
        /// HackRF sample rate, MS/s: 2.048 (DAB's own) or a multiple of it.
        #[arg(long, default_value_t = 4.096)]
        rate: f64,
        /// Seconds to record.
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[command(flatten)]
        gains: Gains,
        /// Output file; by default captures/<block>_<MHz>MHz_<rate>Msps[_amp].cs8.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(clap::Args, Clone, Copy)]
pub struct Gains {
    /// IF (LNA) gain, 0–40 dB in 8 dB steps.
    #[arg(long, default_value_t = 40)]
    pub lna: u16,
    /// Baseband (VGA) gain, 0–62 dB in 2 dB steps.
    #[arg(long, default_value_t = 20)]
    pub vga: u16,
    /// The HackRF's RF amplifier (+14 dB). The antenna-port power stays off.
    #[arg(long)]
    pub amp: bool,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Blocks => {
            for b in BAND_III {
                println!("{:>4}  {:>8.3} MHz", b.name, b.khz as f64 / 1e3);
            }
            Ok(())
        }
        Command::Scan { gains, dwell } => scan(gains, dwell),
        Command::Decode(a) => decode::run(*a),
        Command::Info {
            source,
            seconds,
            gains,
        } => info::run(&source, seconds, gains),
        Command::Capture {
            what,
            rate,
            seconds,
            gains,
            out,
        } => capture(&what, rate, seconds, gains, out),
    }
}

/// A block name or a frequency in MHz, as Hz.
pub fn tuning(what: &str) -> Result<f64> {
    if let Some(b) = parse_block(what) {
        return Ok(b.hz());
    }
    let mhz: f64 = what
        .trim()
        .trim_end_matches("MHz")
        .parse()
        .with_context(|| format!("not a block or a frequency in MHz: {what}"))?;
    Ok(mhz * 1e6)
}

#[cfg(feature = "hackrf")]
pub fn open(hz: f64, rate: f64, g: Gains) -> Result<decdab_io::HackRfSource> {
    use decdab_io::{HackRfGains, HackRfSettings, HackRfSource};
    Ok(HackRfSource::open(HackRfSettings {
        center_hz: hz.round() as u64,
        sample_rate: (rate * 1e6).round() as u32,
        gains: HackRfGains {
            amp: g.amp,
            lna_db: g.lna,
            vga_db: g.vga,
        },
    })?)
}

#[cfg(feature = "hackrf")]
fn capture(what: &str, rate: f64, seconds: f64, g: Gains, out: Option<PathBuf>) -> Result<()> {
    use decdab_core::SampleFormat;
    use decdab_io::{IqFileWriter, IqSource};
    let hz = tuning(what)?;
    let path = out.unwrap_or_else(|| {
        let block = block_at(hz).map_or("f".to_string(), |b| b.name.to_ascii_lowercase());
        PathBuf::from(format!(
            "captures/{block}_{:.3}MHz_{}Msps{}.cs8",
            hz / 1e6,
            rate,
            if g.amp { "_amp" } else { "" }
        ))
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut src = open(hz, rate, g)?;
    let mut w = IqFileWriter::create(&path, SampleFormat::Cs8)?;
    let want = (seconds * rate * 1e6) as usize;
    let (mut n, mut peak, mut pow, mut clip) = (0usize, 0f32, 0f64, 0usize);
    let mut buf = Vec::new();
    let t0 = Instant::now();
    // The first few buffers are the tuner settling.
    for _ in 0..4 {
        src.read(&mut buf)?;
    }
    while n < want {
        src.read(&mut buf)?;
        let take = buf.len().min(want - n);
        for s in &buf[..take] {
            let m = s.re.abs().max(s.im.abs());
            peak = peak.max(m);
            clip += (m >= 127.0 / 128.0) as usize;
            pow += s.norm_sqr() as f64;
        }
        w.write(&buf[..take])?;
        n += take;
    }
    w.finish()?;
    let drops = src.control().overflows();
    println!(
        "{}: {:.1} s in {:.1} s, rms {:.1} dBFS, peak {:.0} %, clipped {:.3} %, dropped buffers {drops}",
        path.display(),
        n as f64 / (rate * 1e6),
        t0.elapsed().as_secs_f64(),
        10.0 * (pow / n as f64).log10(),
        peak * 100.0,
        clip as f64 * 100.0 / n as f64
    );
    // Stop the radio before the process exits (it keeps receiving otherwise).
    drop(src);
    Ok(())
}

/// The averaged power spectrum of `secs` of samples, `n` bins, DC in the
/// middle, dB.
#[cfg(feature = "hackrf")]
fn spectrum(src: &mut dyn decdab_io::IqSource, n: usize, secs: f64) -> Result<Vec<f32>> {
    use num_complex::Complex32;
    use rustfft::FftPlanner;
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let win: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
        .collect();
    let mut acc = vec![0f64; n];
    let want = (secs * src.sample_rate()) as usize;
    let (mut got, mut frames) = (0usize, 0usize);
    let mut buf = Vec::new();
    let mut work = vec![Complex32::default(); n];
    while got < want {
        src.read(&mut buf)?;
        got += buf.len();
        for c in buf.chunks_exact(n) {
            for ((w, s), h) in work.iter_mut().zip(c).zip(&win) {
                *w = s * h;
            }
            fft.process(&mut work);
            for (a, w) in acc.iter_mut().zip(&work) {
                *a += w.norm_sqr() as f64;
            }
            frames += 1;
        }
    }
    let norm = 1.0 / (frames.max(1) as f64 * n as f64 * n as f64 * 0.375);
    Ok((0..n)
        .map(|i| (10.0 * (acc[(i + n / 2) % n] * norm).max(1e-20).log10()) as f32)
        .collect())
}

#[cfg(feature = "hackrf")]
fn scan(g: Gains, dwell: f64) -> Result<()> {
    use decdab_io::IqSource;
    const RATE: f64 = 4.096;
    const N: usize = 4096; // 1 kHz bins
    let blocks: Vec<_> = scan_blocks().collect();
    let mut src = open(blocks[0].hz(), RATE, g)?;
    let bin = RATE * 1e6 / N as f64;
    let band = |db: &[f32], lo: f64, hi: f64| -> f32 {
        // Mean power (linear) over |f| in [lo, hi) Hz, both sides, as dB.
        let (mut s, mut k) = (0f64, 0usize);
        for (i, &d) in db.iter().enumerate() {
            let f = ((i as f64 - (N / 2) as f64) * bin).abs();
            if f >= lo && f < hi {
                s += 10f64.powf(d as f64 / 10.0);
                k += 1;
            }
        }
        (10.0 * (s / k.max(1) as f64).log10()) as f32
    };
    println!("block      MHz   in-band  gap   margin");
    for b in blocks {
        src.retune(b.hz())?;
        let db = spectrum(&mut src, N, dwell)?;
        // In band: the carriers away from DC; the gap: between this block's
        // edge (768 kHz) and the next block's (944 kHz).
        let inb = band(&db, 20e3, 700e3);
        let gap = band(&db, 800e3, 910e3);
        let m = inb - gap;
        println!(
            "{:>4} {:>9.3} {:>8.1} {:>6.1} {:>6.1}{}",
            b.name,
            b.khz as f64 / 1e3,
            inb,
            gap,
            m,
            if m > 6.0 { "  DAB?" } else { "" }
        );
    }
    drop(src);
    Ok(())
}

#[cfg(not(feature = "hackrf"))]
fn scan(_: Gains, _: f64) -> Result<()> {
    bail!("built without the hackrf feature")
}

#[cfg(not(feature = "hackrf"))]
fn capture(_: &str, _: f64, _: f64, _: Gains, _: Option<PathBuf>) -> Result<()> {
    bail!("built without the hackrf feature")
}
