// made by CasualArclamp
//! DecDAB command line.
//!
//! - `blocks`: the Band III block plan.
//! - `scan`: step the radio over Band III and say which blocks carry an
//!   ensemble (a flat 1.536 MHz block with a gap either side).
//! - `capture`: record a block (or a frequency) from the radio to an IQ
//!   file named by its tuning, so the GUI and CLI can read the centre and
//!   rate back from the name.
//! - `decode`: one service to a WAV file, with the decoding figures.
//! - `info`: lock to an ensemble (a capture or a block on the radio) and
//!   list what its FIC says: label, services, sub-channels, the time.
//! - `airspy`: what the Airspy says about itself (board, firmware, rates).
//!
//! The radio is the HackRF One unless `--radio airspy` says otherwise.

mod decode;
mod info;
mod source;

use std::path::PathBuf;
use std::time::Instant;

#[allow(unused_imports)] // `bail` is only used without a radio feature
use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use decdab_core::channel::{BAND_III, block_at, parse_block, scan_blocks};
use decdab_io::AirspyGains;

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
    /// Step the radio over Band III and report each block's level and
    /// whether it looks like a DAB ensemble.
    Scan {
        #[command(flatten)]
        gains: Gains,
        /// Dwell on each block, seconds.
        #[arg(long, default_value_t = 0.3)]
        dwell: f64,
    },
    /// List an ensemble's services from a capture or a block on the radio.
    Info {
        /// An IQ file, or a block ("12B") / frequency in MHz for the radio.
        source: String,
        /// Seconds to listen.
        #[arg(long, default_value_t = 4.0)]
        seconds: f64,
        #[command(flatten)]
        gains: Gains,
    },
    /// Decode one service to a WAV file and report on it.
    Decode(Box<decode::DecodeArgs>),
    /// What the Airspy says about itself: board, firmware, sample rates.
    Airspy,
    /// Record a block ("12B") or a frequency in MHz from the radio.
    Capture {
        /// A Band III block ("12B") or a centre frequency in MHz.
        what: String,
        /// Sample rate, MS/s: 2.048 (DAB's own) or a multiple of it (4.096 and
        /// 8.192 for the Airspy too).
        #[arg(long, default_value_t = 4.096)]
        rate: f64,
        /// Seconds to record.
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[command(flatten)]
        gains: Gains,
        /// Output file; by default captures/<block>_<MHz>MHz_<rate>Msps[_amp].cs8
        /// (HackRF) or ..._airspy.cs16 (Airspy: 16-bit, it has 12 bits).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

/// The radio a live source is read from.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Radio {
    /// HackRF One.
    Hackrf,
    /// Airspy R2 (also the original Airspy).
    Airspy,
}

/// The radio and its gains: `--lna/--vga/--amp` are the HackRF's, the
/// `--airspy-*` ones the Airspy's. Unused by a file.
#[derive(clap::Args, Clone, Copy)]
pub struct Gains {
    /// The radio for a live source.
    #[arg(long, value_enum, default_value = "hackrf")]
    pub radio: Radio,
    /// HackRF: IF (LNA) gain, 0–40 dB in 8 dB steps.
    #[arg(long, default_value_t = 40)]
    pub lna: u16,
    /// HackRF: baseband (VGA) gain, 0–62 dB in 2 dB steps.
    #[arg(long, default_value_t = 20)]
    pub vga: u16,
    /// HackRF: the RF amplifier (+14 dB). The antenna-port power stays off.
    #[arg(long)]
    pub amp: bool,
    /// Airspy: a libairspy gain preset, `sensitivity:N` or `linearity:N`
    /// (N 0–21, 21 the most gain). Default: sensitivity:15.
    #[arg(long, value_parser = AirspyGains::parse_preset, conflicts_with_all = ["airspy_lna", "airspy_mixer", "airspy_vga"])]
    pub airspy_preset: Option<AirspyGains>,
    /// Airspy: LNA gain, 0–14 (default: the preset's).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=14))]
    pub airspy_lna: Option<u8>,
    /// Airspy: mixer gain, 0–15.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=15))]
    pub airspy_mixer: Option<u8>,
    /// Airspy: VGA gain, 0–15.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=15))]
    pub airspy_vga: Option<u8>,
    /// Airspy: 12-bit packed transfers (30 instead of 40 MB/s on the USB).
    #[arg(long)]
    pub airspy_packed: bool,
}

impl Gains {
    /// The Airspy's gains: the preset (or the default one), with any single
    /// stage given on the command line put over it.
    pub fn airspy_gains(&self) -> AirspyGains {
        let base = self.airspy_preset.unwrap_or_default();
        AirspyGains {
            lna: self.airspy_lna.unwrap_or(base.lna),
            mixer: self.airspy_mixer.unwrap_or(base.mixer),
            vga: self.airspy_vga.unwrap_or(base.vga),
        }
    }
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
        Command::Airspy => airspy_info(),
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

/// The radio, tuned to `hz` (Hz) at `rate` (MS/s).
pub fn open(hz: f64, rate: f64, g: Gains) -> Result<source::Live> {
    source::open_live(hz, rate, g)
}

#[cfg(feature = "airspy")]
fn airspy_info() -> Result<()> {
    let i = decdab_io::airspy_info()?;
    println!("board     {} (id {})", i.board_name, i.board_id);
    println!("firmware  {}", i.firmware);
    let rates: Vec<String> = i
        .sample_rates
        .iter()
        .map(|r| format!("{} MS/s", *r as f64 / 1e6))
        .collect();
    println!(
        "rates     {} (DecDAB uses 10 MS/s IQ, 20 MS/s ADC)",
        rates.join(", ")
    );
    Ok(())
}

#[cfg(not(feature = "airspy"))]
fn airspy_info() -> Result<()> {
    bail!("built without the airspy feature")
}

fn capture(what: &str, rate: f64, seconds: f64, g: Gains, out: Option<PathBuf>) -> Result<()> {
    use decdab_core::SampleFormat;
    use decdab_io::{IqFileWriter, IqSource};
    let hz = tuning(what)?;
    // The HackRF's samples are 8-bit; the Airspy's are 12-bit, so 16.
    let (fmt, ext, tag) = match g.radio {
        Radio::Hackrf => (SampleFormat::Cs8, "cs8", if g.amp { "_amp" } else { "" }),
        Radio::Airspy => (SampleFormat::Cs16, "cs16", "_airspy"),
    };
    let path = out.unwrap_or_else(|| {
        let block = block_at(hz).map_or("f".to_string(), |b| b.name.to_ascii_lowercase());
        PathBuf::from(format!(
            "captures/{block}_{:.3}MHz_{}Msps{tag}.{ext}",
            hz / 1e6,
            rate
        ))
    });
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut src = open(hz, rate, g)?;
    let mut w = IqFileWriter::create(&path, fmt)?;
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
    let drops = src.overflows();
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
