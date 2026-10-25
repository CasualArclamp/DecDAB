// made by CasualArclamp
//! The sounder's rows, row by row: the common phase step from the row before
//! (degrees, mean over the carriers) and the mean power (dB), for a few
//! frames. A pattern that repeats each frame shows on the Doppler maps as
//! lines at the frame rate (10.4 Hz).
//!
//! `cargo run --release -p decdab-ofdm --example sound_probe -- CAPTURE [frames]`

use std::path::Path;

use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_ofdm::{Demod, Frontend};
use num_complex::Complex32;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let want: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let hint = name_hint(path);
    let rate = hint.sample_rate.unwrap();
    let mut src = IqFileReader::open(path, format_from_path(path).unwrap(), rate, 1 << 16).unwrap();
    let mut fe = Frontend::new(rate).unwrap();
    let mut demod = Demod::new();
    demod.set_sounding(true);
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let mut prev: Option<Vec<Complex32>> = None;
    let mut done = 0u64;
    while done < want + 20 {
        if src.read(&mut buf).unwrap() == 0 {
            break;
        }
        fe.process(&buf, &mut dec);
        demod.push(&dec, |f| {
            done += 1;
            if f.stats.frame < 20 {
                return; // settled first
            }
            let mut line = format!("frame {:3} step {:+}:", f.stats.frame, f.stats.timing_step);
            for row in f.sound {
                let p = row.iter().map(|z| z.norm_sqr()).sum::<f32>() / row.len() as f32;
                let dphi = prev.as_ref().map_or(0.0, |q| {
                    let c: Complex32 = row.iter().zip(q).map(|(a, b)| a * b.conj()).sum();
                    c.arg().to_degrees()
                });
                line += &format!("  {dphi:+6.1}°/{:5.1}dB", 10.0 * p.log10());
                prev = Some(row.clone());
            }
            println!("{line}");
        });
    }
}
