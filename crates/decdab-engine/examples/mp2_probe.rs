// made by CasualArclamp
//! An MP2 service's frames under added noise: dumps the raw Layer II
//! frames (for checking the CRC rules by hand).
//!
//! `cargo run --release -p decdab-engine --example mp2_probe -- CAPTURE SERVICE SECONDS DB OUT.mp2`
//! where DB is the added noise against the capture's power (-99 for none).

use std::io::Write;
use std::path::Path;

use decdab_core::Iq;
use decdab_fic::FicDecoder;
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_msc::SubChannelDecoder;
use decdab_ofdm::params::{CIF_BITS, CIFS, FIC_BITS};
use decdab_ofdm::{Demod, Frontend};

/// A small Gaussian source (xorshift and Box–Muller), as in `threshold`.
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64).max(1e-300)
    }

    fn complex(&mut self, sigma: f32) -> Iq {
        let r = (-2.0 * self.uniform().ln()).sqrt() as f32 * sigma;
        let t = std::f64::consts::TAU * self.uniform();
        Iq::from_polar(r, t as f32)
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let pattern = a[2].to_lowercase();
    let secs: f64 = a[3].parse().unwrap();
    let db: f64 = a[4].parse().unwrap();
    let mut out = std::fs::File::create(&a[5]).unwrap();
    let rate = name_hint(path).sample_rate.unwrap();
    let fmt = format_from_path(path).unwrap();
    let mut src = IqFileReader::open(path, fmt, rate, 1 << 16).unwrap();
    let mut fe = Frontend::new(rate).unwrap();
    let mut demod = Demod::new();
    let mut fic = FicDecoder::new();
    let mut sub: Option<SubChannelDecoder> = None;
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let mut sigma: Option<f32> = None;
    let mut read = 0usize;
    let mut frames: Vec<Vec<i8>> = Vec::new();
    while (read as f64) < secs * rate {
        if src.read(&mut buf).unwrap() == 0 {
            break;
        }
        read += buf.len();
        let s = *sigma.get_or_insert_with(|| {
            let p = buf.iter().map(|z| z.norm_sqr()).sum::<f32>() / buf.len() as f32;
            (p * 10f32.powf(db as f32 / 10.0)).sqrt() / std::f32::consts::SQRT_2
        });
        if db > -90.0 {
            for z in buf.iter_mut() {
                *z += noise.complex(s);
            }
        }
        fe.process(&buf, &mut dec);
        demod.push(&dec, |f| frames.push(f.soft.to_vec()));
        for soft in frames.drain(..) {
            fic.push_frame(&soft[..FIC_BITS]);
            if sub.is_none() {
                let e = &fic.ensemble;
                if let Some(s) = e
                    .services_by_name()
                    .into_iter()
                    .find(|s| s.name().to_lowercase().contains(&pattern))
                    && let Some(c) = s.primary()
                    && let Some(sc) = e.subchannel_of(c)
                {
                    eprintln!("{} — {}", s.name(), sc.protection_name());
                    sub = SubChannelDecoder::new(*sc);
                }
            }
            if let Some(d) = &mut sub {
                for c in 0..CIFS {
                    let at = FIC_BITS + c * CIF_BITS;
                    if let Some(lf) = d.push_cif(&soft[at..at + CIF_BITS]) {
                        out.write_all(lf).unwrap();
                    }
                }
            }
        }
    }
}
