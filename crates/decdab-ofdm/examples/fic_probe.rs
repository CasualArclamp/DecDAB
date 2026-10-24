// made by CasualArclamp
//! Demodulate a capture and decode its FIC: per-frame lock figures and how
//! many FIBs pass their CRC.
//!
//! `cargo run --release -p decdab-ofdm --example fic_probe -- captures/12b_225.648MHz_4.096Msps_amp.cs8 [seconds]`

use std::path::Path;
use std::time::Instant;

use decdab_core::crc::crc16_ok;
use decdab_fec::prbs::Prbs;
use decdab_fec::{Plan, Viterbi};
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_ofdm::params::{FIC_BITS, FIC_BLOCK_BITS};
use decdab_ofdm::{Demod, Frontend};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = Path::new(&args[1]);
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1e9);
    let hint = name_hint(path);
    let rate = hint.sample_rate.unwrap_or(2.048e6);
    let fmt = format_from_path(path).unwrap_or(decdab_core::SampleFormat::Cs8);
    let mut src = IqFileReader::open(path, fmt, rate, 1 << 16).expect("open");
    let mut fe = Frontend::new(rate).expect("rate");
    let mut demod = Demod::new();
    let plan = Plan::fic();
    let mut vit = Viterbi::new();
    let prbs = Prbs::default();
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let (mut fibs, mut good) = (0usize, 0usize);
    let mut mother = Vec::new();
    let mut bits = Vec::new();
    let mut read = 0usize;
    let t0 = Instant::now();
    let mut t_fic = 0f64;
    while read < (secs * rate) as usize {
        let n = src.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        read += n;
        fe.process(&buf, &mut dec);
        demod.push(&dec, |f| {
            let t1 = Instant::now();
            let fic = &f.soft[..FIC_BITS];
            let mut ok = 0;
            for blk in fic.chunks(FIC_BLOCK_BITS) {
                plan.depuncture(blk, &mut mother);
                bits.clear();
                vit.decode(&mother, &mut bits);
                prbs.apply(&mut bits);
                for fib in bits.chunks(256) {
                    let bytes: Vec<u8> = fib
                        .chunks(8)
                        .map(|b| b.iter().fold(0u8, |a, &x| (a << 1) | x))
                        .collect();
                    fibs += 1;
                    if crc16_ok(&bytes) {
                        ok += 1;
                        good += 1;
                    }
                }
            }
            t_fic += t1.elapsed().as_secs_f64();
            let s = f.stats;
            println!(
                "frame {:4}  cfo {:+9.1} Hz  step {:+4}  peak {:5.1} dB  MER {:5.1} dB  FIBs {ok}/12",
                s.frame, s.cfo_hz, s.timing_step, s.peak_db, s.mer_db
            );
        });
    }
    let dt = t0.elapsed().as_secs_f64();
    let audio = read as f64 / rate;
    println!(
        "{:.1} s of signal in {:.2} s ({:.1} % of a core; FIC decoding {:.1} %); FIBs {good}/{fibs} good",
        audio,
        dt,
        dt / audio * 100.0,
        t_fic / audio * 100.0
    );
}
