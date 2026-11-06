// made by CasualArclamp
//! Passive radar from a capture: each range–Doppler map as a PNG (Doppler
//! across, ±400 Hz, grid every 100 Hz; bistatic range up, grid every
//! 25 km; colour 0–15 dB above the noise floor), with its figures and
//! strongest echoes printed.
//!
//! `cargo run --release -p decdab-gui --example radar_probe -- CAPTURE [out-dir] [max-km]`

use std::path::{Path, PathBuf};
use std::time::Instant;

use decdab_engine::radar::{CPI_SLOTS, RadarHistory, RangeDoppler, echoes};
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};
use decdab_ofdm::{Demod, Frontend, RADAR_BIN_M, RADAR_BINS, RADAR_PRE, RADAR_SLOT_S};

fn colour(v: f32) -> [u8; 3] {
    // Dark blue → purple → orange → pale yellow over 0…1.
    const STOPS: [[f32; 3]; 5] = [
        [8.0, 8.0, 30.0],
        [70.0, 20.0, 110.0],
        [190.0, 50.0, 90.0],
        [250.0, 150.0, 40.0],
        [255.0, 250.0, 190.0],
    ];
    let x = v.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let i = (x.floor() as usize).min(STOPS.len() - 2);
    let f = x - i as f32;
    std::array::from_fn(|c| (STOPS[i][c] + f * (STOPS[i + 1][c] - STOPS[i][c])) as u8)
}

fn save(m: &RangeDoppler, ranges: usize, path: &Path) {
    let (w, h) = (m.dopplers as u32, ranges as u32);
    let mut img = image::RgbImage::new(w, h);
    for r in 0..ranges {
        for d in 0..m.dopplers {
            let mut c = colour(m.at(r, d) / 15.0);
            let hz = m.doppler_hz(d);
            let on_hz = (hz / 100.0).round() * 100.0;
            let km = m.range_km(r);
            let on_km = (km / 25.0).round() * 25.0;
            if (hz - on_hz).abs() < m.doppler_step_hz / 2.0
                || (km - on_km).abs() < m.range_step_km / 2.0
            {
                c = c.map(|v| v.saturating_add(45));
            }
            img.put_pixel(d as u32, h - 1 - r as u32, image::Rgb(c));
        }
    }
    img.save(path).unwrap();
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let path = Path::new(&a[1]);
    let out = PathBuf::from(a.get(2).map_or("out/radar", |s| s.as_str()));
    let max_km: f64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(100.0);
    std::fs::create_dir_all(&out).unwrap();
    let bin_km = RADAR_BIN_M / 1000.0;
    let ranges = ((max_km / bin_km).ceil() as usize).min(RADAR_BINS - RADAR_PRE);

    let hint = name_hint(path);
    let rate = hint.sample_rate.unwrap();
    let mut src = IqFileReader::open(path, format_from_path(path).unwrap(), rate, 1 << 16).unwrap();
    let mut fe = Frontend::new(rate).unwrap();
    let mut demod = Demod::new();
    demod.set_radar(true);
    let mut hist = RadarHistory::new(CPI_SLOTS, RADAR_SLOT_S, bin_km, RADAR_PRE + ranges, ranges);
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let (mut n, mut demod_s, mut map_s, mut frames) = (0, 0.0, 0.0, 0u64);
    let (mut ridge_sum, mut comb_sum, mut ridge_comb_sum) = (0.0, 0.0, 0.0);
    loop {
        if src.read(&mut buf).unwrap() == 0 {
            break;
        }
        fe.process(&buf, &mut dec);
        let t0 = Instant::now();
        let mut maps = Vec::new();
        let mut t_map = 0.0;
        demod.push(&dec, |f| {
            frames += 1;
            let t = Instant::now();
            if hist.push_frame(f.radar)
                && let Some(m) = hist.map()
            {
                maps.push((m, f.stats.mer_db));
            }
            t_map += t.elapsed().as_secs_f64();
        });
        demod_s += t0.elapsed().as_secs_f64() - t_map;
        map_s += t_map;
        for (m, mer) in maps {
            let file = out.join(format!("map_{n:03}.png"));
            let ranges = m.ranges;
            save(&m, ranges, &file);
            println!(
                "map {n:3}  t {:5.1} s  MER {mer:4.1} dB  floor {:6.1} dB under the direct path  max {:4.1} dB",
                frames as f64 * 0.096,
                m.floor_db,
                m.max_snr_db
            );
            for e in echoes(&m, 15.0, max_km, 12.0, 6) {
                println!(
                    "          {:6.2} km  {:+7.1} Hz  {:4.1} dB",
                    e.range_km, e.doppler_hz, e.db
                );
            }
            // The clutter ridge: mean dB at 5–40 Hz against beyond 100 Hz,
            // over 20–100 km; and the frame comb at 0–1 km: ±10.4·k Hz,
            // k = 2…20, against beside it.
            let band = |r0: f64, r1: f64, f: &dyn Fn(f64) -> bool| {
                let (mut s, mut c) = (0f64, 0usize);
                for r in 0..ranges {
                    let km = m.range_km(r);
                    if km >= r0 && km < r1 {
                        for d in 0..m.dopplers {
                            if f(m.doppler_hz(d)) {
                                s += m.at(r, d) as f64;
                                c += 1;
                            }
                        }
                    }
                }
                s / c.max(1) as f64
            };
            let ridge = band(20.0, 100.0, &|h| (5.0..40.0).contains(&h.abs()))
                - band(20.0, 100.0, &|h| h.abs() > 100.0);
            let comb = |h: f64| {
                let k = (h.abs() / 10.39).round();
                (2.0..=20.0).contains(&k) && (h.abs() - k * 10.39).abs() < 0.6
            };
            let off = |h: f64| {
                let k = (h.abs() / 10.39).round();
                (2.0..=20.0).contains(&k) && ((h.abs() - k * 10.39).abs() - 4.0).abs() < 1.0
            };
            let comb_db = band(0.0, 1.0, &comb) - band(0.0, 1.0, &off);
            // The comb over the ridge's ranges, k = 1…4 (within ±42 Hz).
            let comb4 = |h: f64| {
                let k = (h.abs() / 10.39).round();
                (1.0..=4.0).contains(&k) && (h.abs() - k * 10.39).abs() < 0.6
            };
            let off4 = |h: f64| {
                let k = (h.abs() / 10.39).round();
                (1.0..=4.0).contains(&k) && ((h.abs() - k * 10.39).abs() - 4.0).abs() < 1.0
            };
            let ridge_comb = band(2.0, 40.0, &comb4) - band(2.0, 40.0, &off4);
            ridge_comb_sum += ridge_comb;
            println!(
                "          ridge {ridge:4.1} dB  frame comb {comb_db:4.1} dB  on the ridge {ridge_comb:4.1} dB"
            );
            ridge_sum += ridge;
            comb_sum += comb_db;
            n += 1;
        }
    }
    let secs = frames as f64 * 0.096;
    println!(
        "mean over the maps: ridge {:.1} dB, frame comb {:.1} dB, on the ridge {:.1} dB",
        ridge_sum / n.max(1) as f64,
        comb_sum / n.max(1) as f64,
        ridge_comb_sum / n.max(1) as f64
    );
    println!(
        "{frames} frames ({secs:.1} s): demod with radar {:.1} % of a core, maps {:.1} %",
        100.0 * demod_s / secs,
        100.0 * map_s / secs
    );
}
