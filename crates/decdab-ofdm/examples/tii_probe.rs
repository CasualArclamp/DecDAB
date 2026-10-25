// made by CasualArclamp
//! The transmitters a capture hears (TII): main and sub identifier, level
//! against the strongest, over the noise, and arrival against the strongest.
//!
//! `cargo run --release -p decdab-ofdm --example tii_probe -- CAPTURE [seconds]`

use std::path::Path;

use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_ofdm::{Demod, Frontend};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let secs: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let rate = name_hint(path).sample_rate.unwrap();
    let mut src = IqFileReader::open(path, format_from_path(path).unwrap(), rate, 1 << 16).unwrap();
    let mut fe = Frontend::new(rate).unwrap();
    let mut demod = Demod::new();
    demod.set_tii(true);
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let mut read = 0usize;
    let mut last = None;
    while (read as f64) < secs * rate {
        let n = src.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        read += n;
        fe.process(&buf, &mut dec);
        demod.push(&dec, |f| {
            if let Some(r) = f.tii {
                last = Some(r.clone());
                let t: Vec<String> = r
                    .transmitters
                    .iter()
                    .map(|t| format!("{:02}/{:02} {:+5.1} dB ({:4.1}) {:+6.1} µs", t.main, t.sub, t.rel_db, t.snr_db, t.delay_us))
                    .collect();
                println!("frame {:4}: {}", f.stats.frame, t.join("  |  "));
            }
        });
    }
    if let Some(r) = last {
        println!("groups over the noise, dB (comb: b0..b7):");
        for (c, g) in r.groups_db.iter().enumerate() {
            let s: Vec<String> = g.iter().map(|x| format!("{x:5.1}")).collect();
            println!("  {c:2}: {}", s.join(" "));
        }
    }
}
