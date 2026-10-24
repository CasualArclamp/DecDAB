// made by CasualArclamp
//! Where decoding breaks, in the receiver's own MER: add white noise to a
//! capture in steps and report, at each, the demodulator's MER, the FIBs
//! lost and the chosen DAB+ service's AUs lost. The card's margin is the MER
//! less the MER at which a code rate stops being error free (see
//! `apps/decdab-gui/src/card.rs`).
//!
//! `cargo run --release -p decdab-engine --example threshold -- CAPTURE SERVICE SECONDS DB...`
//! where each DB is the added noise against the capture's power (e.g. -14).

use std::path::Path;

use decdab_codecs::Pcm;
use decdab_core::Iq;
use decdab_data::pad::PadEvent;
use decdab_engine::ServiceDecoder;
use decdab_fic::FicDecoder;
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_ofdm::params::{CIF_BITS, CIFS, FIC_BITS};
use decdab_ofdm::{Demod, Frontend};

/// A small Gaussian source (xorshift and Box–Muller): no crate needed.
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
    let levels: Vec<f64> = a[4..].iter().map(|s| s.parse().unwrap()).collect();
    let hint = name_hint(path);
    let rate = hint.sample_rate.unwrap();
    let fmt = format_from_path(path).unwrap();
    println!("added dB   MER dB   FIBs lost   AUs lost / all   RS lost");
    for &db in std::iter::once(&-99.0).chain(&levels) {
        let mut src = IqFileReader::open(path, fmt, rate, 1 << 16).unwrap();
        let mut fe = Frontend::new(rate).unwrap();
        let mut demod = Demod::new();
        let mut fic = FicDecoder::new();
        let mut svc: Option<ServiceDecoder> = None;
        let (mut buf, mut dec) = (Vec::new(), Vec::new());
        let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
        let mut sigma: Option<f32> = None;
        let (mut read, mut mer_sum, mut mer_n) = (0usize, 0f64, 0u32);
        let (mut pcm, mut pad): (Vec<Pcm>, Vec<PadEvent>) = (Vec::new(), Vec::new());
        let mut frames: Vec<Vec<i8>> = Vec::new();
        while (read as f64) < secs * rate {
            if src.read(&mut buf).unwrap() == 0 {
                break;
            }
            read += buf.len();
            // The noise is set against the first block's power.
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
            demod.push(&dec, |f| {
                frames.push(f.soft.to_vec());
                // Past the first second, the lock has settled.
                if f.stats.frame > 10 {
                    mer_sum += f.stats.mer_db as f64;
                    mer_n += 1;
                }
            });
            for soft in frames.drain(..) {
                fic.push_frame(&soft[..FIC_BITS]);
                if svc.is_none() {
                    let e = &fic.ensemble;
                    if let Some(s) = e
                        .services_by_name()
                        .into_iter()
                        .find(|s| s.name().to_lowercase().contains(&pattern))
                        && let Some(c) = s.primary()
                        && let Some(sc) = e.subchannel_of(c)
                    {
                        svc = ServiceDecoder::new(c, *sc);
                    }
                }
                if let Some(d) = &mut svc {
                    for c in 0..CIFS {
                        let at = FIC_BITS + c * CIF_BITS;
                        d.push_cif(&soft[at..at + CIF_BITS], &mut pcm, &mut pad);
                    }
                    pcm.clear();
                    pad.clear();
                }
            }
        }
        let fs = fic.stats;
        let (au_bad, au_all, rs) = svc.as_ref().map_or((0, 0, 0), |d| {
            (d.stats.au_bad, d.stats.au_ok + d.stats.au_bad, d.stats.rs_failed)
        });
        println!(
            "{:>8}  {:>7.2}  {:>5} / {:<5}  {:>6} / {:<6}  {:>5}{}",
            if db < -90.0 {
                "none".to_string()
            } else {
                format!("{db:.1}")
            },
            mer_sum / mer_n.max(1) as f64,
            fs.fibs - fs.fibs_ok,
            fs.fibs,
            au_bad,
            au_all,
            rs,
            svc.as_ref()
                .map_or(String::new(), |d| format!("   {}", d.subchannel().protection_name()))
        );
    }
}
