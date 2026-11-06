// made by CasualArclamp
//! Passive radar: the range–Doppler map of what reflects the ensemble's
//! signal, from the demodulator's range profiles (`Frame::radar`, one a
//! symbol).
//!
//! Each profile is the channel's impulse response at one moment: the direct
//! path at delay 0, and every echo at its excess delay (bistatic range =
//! c·τ: the path via the reflector less the direct path). Over a coherent
//! processing interval (CPI) of about a second, each delay bin's values are
//! a time series; its FFT is the Doppler spectrum at that range. Before it:
//!
//! - each profile is turned by the direct path's phase and scaled by its
//!   level, so 0 Hz is the direct signal's frequency (bistatic Doppler is
//!   relative to it) and the receiver's own phase and gain wander, common
//!   to every path, cancel;
//! - each bin's mean over the CPI is taken out: everything that does not
//!   move (the direct path, buildings, hills), which would otherwise leak
//!   over the whole Doppler axis, leaving the zero-Doppler line empty;
//! - in the strong bins (the direct path, the SFN's other transmitters and
//!   the big static echoes, [`STRONG_DB`] over the typical bin) the mean is
//!   taken per slot of the frame rather than over all: whatever the frame
//!   does to them alike each frame (the phase reference's exact estimate
//!   against the decided ones, the null's stand-in slot, the timing's
//!   steps) goes with it, which would otherwise copy them at every
//!   multiple of the frame rate (10.4 Hz) right across the map.
//!
//! The method after Juha Vierinen's FM passive radar and the OFDM
//! reciprocal-filter literature (e.g. Searle, Howard & Palmer, "Evaluating
//! the use of DAB+ for passive radar", 2015); the code is new.

use std::collections::VecDeque;
use std::f64::consts::PI;
use std::sync::Arc;

use decdab_core::Iq;
use rustfft::{Fft, FftPlanner};

/// Slots (symbols and nulls) in a CPI: ten frames, 0.96 s.
pub const CPI_SLOTS: usize = 770;
/// A new map every this many frames (overlapping CPIs, ~0.48 s).
pub const EVERY_FRAMES: usize = 5;
/// Slots a frame: the period of the frame-locked pattern.
pub const FRAME_SLOTS: usize = 77;
/// A bin this far over the median bin (mean power over the CPI, dB) has
/// its frame-locked pattern taken out, not just its mean.
pub const STRONG_DB: f32 = 25.0;

/// The range–Doppler map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RangeDoppler {
    /// Power, dB above the noise floor: `ranges` rows of `dopplers` values,
    /// the nearest range first, the lowest Doppler first in each.
    pub db: Vec<f32>,
    pub ranges: usize,
    pub dopplers: usize,
    /// Bistatic range of a row, km (row 0 is 0 km).
    pub range_step_km: f64,
    pub doppler_start_hz: f64,
    pub doppler_step_hz: f64,
    pub cpi_s: f64,
    /// The noise floor (the map's median) against the direct path, dB:
    /// how far down an echo can be seen.
    pub floor_db: f32,
    /// The strongest point off the zero-Doppler line, dB above the floor.
    pub max_snr_db: f32,
}

impl RangeDoppler {
    pub fn at(&self, range: usize, doppler: usize) -> f32 {
        self.db[range * self.dopplers + doppler]
    }

    pub fn range_km(&self, r: usize) -> f64 {
        r as f64 * self.range_step_km
    }

    pub fn doppler_hz(&self, d: usize) -> f64 {
        self.doppler_start_hz + d as f64 * self.doppler_step_hz
    }
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (2.0 * PI * (i as f64 + 0.5) / n as f64).cos()) as f32)
        .collect()
}

/// The map from `rows` (range profiles `slot_s` apart, the same number of
/// bins each, `bin_km` of bistatic range a bin), `ranges` bins from the
/// strongest path (range 0; bins before it are left out): `None` for too
/// few rows or nothing in them.
pub fn range_doppler(
    rows: &[Vec<Iq>],
    slot_s: f64,
    bin_km: f64,
    ranges: usize,
) -> Option<RangeDoppler> {
    let t_len = rows.len();
    let bins = rows.first()?.len();
    if t_len < 64 || bins == 0 || rows.iter().any(|r| r.len() != bins) {
        return None;
    }
    // The direct path: the strongest bin on average.
    let mut mean_p = vec![0f32; bins];
    for row in rows {
        for (m, z) in mean_p.iter_mut().zip(row) {
            *m += z.norm_sqr();
        }
    }
    let (ref_bin, _) = mean_p
        .iter()
        .enumerate()
        .fold((0, 0f32), |b, (i, &p)| if p > b.1 { (i, p) } else { b });
    // Each row's turn and scale: the direct path's phase undone, its level
    // made the mean level (a deep fade of the direct path is not divided
    // into: the scale is capped at 10×).
    let mean_a = rows.iter().map(|r| r[ref_bin].norm()).sum::<f32>() / t_len as f32;
    if mean_a <= 0.0 || !mean_a.is_finite() {
        return None;
    }
    let g: Vec<Iq> = rows
        .iter()
        .map(|r| {
            let c = r[ref_bin];
            let a = c.norm().max(0.1 * mean_a);
            c.conj() * (mean_a / (a * a))
        })
        .collect();

    let win = hann(t_len);
    let nf = t_len.next_power_of_two();
    let fft: Arc<dyn Fft<f32>> = FftPlanner::<f32>::new().plan_fft_forward(nf);
    let mut scratch = vec![Iq::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    let mut col = vec![Iq::new(0.0, 0.0); nf];
    let ranges = ranges.min(bins - ref_bin);
    let mut power = vec![0f32; ranges * nf];
    let win_sum: f32 = win.iter().sum();
    let direct = (mean_a * win_sum).powi(2);
    let strong = {
        let mut m = mean_p.clone();
        let mid = m.len() / 2;
        let (_, &mut med, _) = m.select_nth_unstable_by(mid, f32::total_cmp);
        med * 10f32.powf(STRONG_DB / 10.0)
    };
    // Frame-locked means need whole frames' worth of each slot.
    let periodic = t_len >= 4 * FRAME_SLOTS;
    let mut per = [Iq::new(0.0, 0.0); FRAME_SLOTS];
    let mut cnt = [0f32; FRAME_SLOTS];
    for (t, c) in cnt.iter_mut().enumerate() {
        *c = t_len.saturating_sub(t).div_ceil(FRAME_SLOTS) as f32;
    }
    for (r, b) in (ref_bin..ref_bin + ranges).enumerate() {
        // The bin's series, what stands still taken out.
        for (t, row) in rows.iter().enumerate() {
            col[t] = row[b] * g[t];
        }
        if periodic && mean_p[b] > strong {
            per.fill(Iq::new(0.0, 0.0));
            for (t, c) in col[..t_len].iter().enumerate() {
                per[t % FRAME_SLOTS] += c;
            }
            for (p, n) in per.iter_mut().zip(&cnt) {
                *p /= *n;
            }
            for (t, (c, &w)) in col[..t_len].iter_mut().zip(&win).enumerate() {
                *c = (*c - per[t % FRAME_SLOTS]) * w;
            }
        } else {
            let mean = col[..t_len].iter().sum::<Iq>() / t_len as f32;
            for (c, &w) in col[..t_len].iter_mut().zip(&win) {
                *c = (*c - mean) * w;
            }
        }
        col[t_len..].fill(Iq::new(0.0, 0.0));
        fft.process_with_scratch(&mut col, &mut scratch);
        // Lowest Doppler first: bins nf/2 … nf−1, then 0 … nf/2−1.
        let out = &mut power[r * nf..(r + 1) * nf];
        for (d, o) in out.iter_mut().enumerate() {
            *o = col[(d + nf / 2) % nf].norm_sqr();
        }
    }
    // The noise floor: the median.
    let mut sorted = power.clone();
    let mid = sorted.len() / 2;
    let (_, &mut floor, _) = sorted.select_nth_unstable_by(mid, f32::total_cmp);
    if floor <= 0.0 || !floor.is_finite() {
        return None;
    }
    // Each Doppler's own floor too: its median over the ranges (every
    // fourth is plenty), where that is higher. Whatever is the same at every
    // range (a weak tone in the band, the radio's DC) is a line up the whole
    // map at its Doppler; against its own column's median it is flat. An
    // echo is at a few ranges and stands out still. (From DecDVB-T's.)
    let col_floor: Vec<f32> = if ranges >= 64 {
        let mut col = vec![0f32; ranges.div_ceil(4)];
        (0..nf)
            .map(|d| {
                for (j, c) in col.iter_mut().enumerate() {
                    *c = power[4 * j * nf + d];
                }
                let mid = col.len() / 2;
                let (_, &mut m, _) = col.select_nth_unstable_by(mid, f32::total_cmp);
                m.max(floor)
            })
            .collect()
    } else {
        vec![floor; nf]
    };
    let doppler_step_hz = 1.0 / (nf as f64 * slot_s);
    let doppler_start_hz = -((nf / 2) as f64) * doppler_step_hz;
    // The zero-Doppler line (±2 Hz) is clutter's leftovers, not targets.
    let near_zero = (2.0 / doppler_step_hz).ceil() as usize;
    let mut max_snr = 0f32;
    let db: Vec<f32> = power
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            let v = 10.0 * (p / col_floor[i % nf]).max(1e-6).log10();
            if (i % nf).abs_diff(nf / 2) > near_zero {
                max_snr = max_snr.max(v);
            }
            v
        })
        .collect();
    Some(RangeDoppler {
        db,
        ranges,
        dopplers: nf,
        range_step_km: bin_km,
        doppler_start_hz,
        doppler_step_hz,
        cpi_s: t_len as f64 * slot_s,
        floor_db: 10.0 * (floor / direct).log10(),
        max_snr_db: max_snr,
    })
}

/// An echo off the clutter ridge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Echo {
    pub range_km: f64,
    pub doppler_hz: f64,
    /// dB above the noise floor.
    pub db: f32,
}

/// The bins nearest the direct path that echoes are not looked for in
/// (0.4 km): its own skirt, and whatever modulates it, are not echoes.
pub const ECHO_FROM_BIN: usize = 3;

/// The strongest points of `m` beyond ±`ridge_hz`, from [`ECHO_FROM_BIN`]
/// out to `max_km`, at least `min_db` over the floor, local peaks (none
/// other within 4 bins each way stronger), strongest first, at most `n`.
pub fn echoes(m: &RangeDoppler, ridge_hz: f64, max_km: f64, min_db: f32, n: usize) -> Vec<Echo> {
    let zero = m.dopplers / 2;
    let near = (ridge_hz / m.doppler_step_hz).ceil() as usize;
    let ranges = ((max_km / m.range_step_km).ceil() as usize).min(m.ranges);
    let mut pts: Vec<(usize, usize, f32)> = Vec::new();
    for r in ECHO_FROM_BIN.min(ranges)..ranges {
        for d in 0..m.dopplers {
            let v = m.at(r, d);
            if d.abs_diff(zero) > near && v >= min_db {
                pts.push((r, d, v));
            }
        }
    }
    // Strongest first; each kept unless a stronger one is already near it.
    pts.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut out: Vec<(usize, usize, f32)> = Vec::new();
    for p in pts {
        if out
            .iter()
            .all(|q| q.0.abs_diff(p.0) > 4 || q.1.abs_diff(p.1) > 4)
        {
            out.push(p);
            if out.len() == n {
                break;
            }
        }
    }
    out.iter()
        .map(|&(r, d, db)| Echo {
            range_km: m.range_km(r),
            doppler_hz: m.doppler_hz(d),
            db,
        })
        .collect()
}

/// The slots of the last CPI, and the newest map.
pub struct RadarHistory {
    rows: VecDeque<Vec<Iq>>,
    cpi: usize,
    slot_s: f64,
    bin_km: f64,
    /// Bins kept a profile, and the map's ranges (from the strongest path).
    bins: usize,
    ranges: usize,
    frames: usize,
    last: Option<Vec<Iq>>,
}

impl RadarHistory {
    /// A history of `cpi` slots `slot_s` apart, keeping the first `bins`
    /// bins (`bin_km` each) of each profile, for maps of `ranges` bins from
    /// the strongest path.
    pub fn new(cpi: usize, slot_s: f64, bin_km: f64, bins: usize, ranges: usize) -> RadarHistory {
        RadarHistory {
            rows: VecDeque::with_capacity(cpi + 80),
            cpi,
            slot_s,
            bin_km,
            bins,
            ranges,
            frames: 0,
            last: None,
        }
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.last = None;
        self.frames = 0;
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Add a frame's profiles (the phase reference first). The null before
    /// it has none: its slot is the mean of the slots either side, so the
    /// series has no hole (a hole each frame would make copies of every
    /// echo at multiples of the frame rate, 10.4 Hz). True when a map is
    /// due ([`RadarHistory::map`]): every [`EVERY_FRAMES`] frames once a CPI
    /// is full.
    pub fn push_frame(&mut self, profiles: &[Vec<Iq>]) -> bool {
        let Some(first) = profiles.first() else {
            return false;
        };
        let n = self.bins.min(first.len());
        // `if let Some(prev) = …`: runs only when there was a frame before
        // (`prev` borrows the Vec inside the Option).
        if let Some(prev) = self.last.as_ref() {
            let null: Vec<Iq> = prev
                .iter()
                .zip(&first[..n])
                .map(|(a, b)| (a + b) * 0.5)
                .collect();
            self.rows.push_back(null);
        }
        for p in profiles {
            let mut r = if self.rows.len() >= self.cpi {
                self.rows.pop_front().unwrap_or_default()
            } else {
                Vec::new()
            };
            r.clear();
            r.extend_from_slice(&p[..n]);
            self.rows.push_back(r);
        }
        while self.rows.len() > self.cpi {
            self.rows.pop_front();
        }
        self.last = profiles.last().map(|p| p[..n].to_vec());
        self.frames += 1;
        self.rows.len() >= self.cpi && self.frames.is_multiple_of(EVERY_FRAMES)
    }

    /// The map of the slots held (the last CPI's), when there are enough.
    pub fn map(&mut self) -> Option<RangeDoppler> {
        range_doppler(
            self.rows.make_contiguous(),
            self.slot_s,
            self.bin_km,
            self.ranges,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOT: f64 = 196_608.0 / 77.0 / 2.048e6;
    const BIN_KM: f64 = 299_792.458 / 2.048e6;

    /// Profiles of a direct path (bin 0, gain 1, wandering in phase as a
    /// receiver's would) and an echo at `bin`, `gain`, `hz`.
    fn rows(n: usize, bin: usize, gain: f32, hz: f64) -> Vec<Vec<Iq>> {
        (0..n)
            .map(|t| {
                let wander = 0.3 * (t as f64 * 0.01).sin() + 2.0 * PI * 3.0 * t as f64 * SLOT;
                let common = Iq::from_polar(1.0, wander as f32);
                let mut r = vec![Iq::new(0.0, 0.0); 256];
                r[0] = common;
                r[1] = common * 0.3; // a static echo: clutter
                r[bin] += common * Iq::from_polar(gain, (2.0 * PI * hz * t as f64 * SLOT) as f32);
                // A little noise, the same every run (a hash of the cell: a
                // sum linear in t would be a tone, not noise).
                for (i, z) in r.iter_mut().enumerate() {
                    let u = |salt: u64| {
                        let mut x = (t as u64) << 32 ^ (i as u64) << 8 ^ salt;
                        x = (x ^ (x >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
                        x = (x ^ (x >> 33)).wrapping_mul(0xc4ce_b9fe_1a85_ec53);
                        ((x ^ (x >> 33)) >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                    };
                    *z += Iq::new(u(1), u(2)) * 1e-3;
                }
                r
            })
            .collect()
    }

    fn peak(m: &RangeDoppler) -> (f64, f64, f32) {
        let (mut best, mut at) = (f32::MIN, (0, 0));
        for r in 0..m.ranges {
            for d in 0..m.dopplers {
                if m.at(r, d) > best {
                    best = m.at(r, d);
                    at = (r, d);
                }
            }
        }
        (m.range_km(at.0), m.doppler_hz(at.1), best)
    }

    #[test]
    fn an_echo_shows_at_its_range_and_doppler() {
        // −40 dB, 120 bins (17.6 km), +85 Hz; clutter and the receiver's
        // wander are taken out.
        let m = range_doppler(&rows(770, 120, 0.01, 85.0), SLOT, BIN_KM, 256).unwrap();
        let (km, hz, db) = peak(&m);
        assert!((km - 120.0 * BIN_KM).abs() < 0.01, "{km} km");
        assert!((hz - 85.0).abs() < 1.5, "{hz} Hz");
        assert!(db > 30.0 && (m.max_snr_db - db).abs() < 1e-3, "{db} dB");
        assert!((m.cpi_s - 770.0 * SLOT).abs() < 1e-9);
        assert!(m.doppler_start_hz < -400.0 && m.doppler_start_hz > -402.0);
        // The direct path and the static echo are gone from 0 Hz.
        let zero = m.dopplers / 2;
        assert!(m.at(0, zero) < db - 20.0 && m.at(1, zero) < db - 20.0);
    }

    #[test]
    fn a_receding_echo_is_negative() {
        let m = range_doppler(&rows(770, 40, 0.01, -150.0), SLOT, BIN_KM, 256).unwrap();
        let (_, hz, _) = peak(&m);
        assert!((hz + 150.0).abs() < 1.5, "{hz} Hz");
    }

    #[test]
    fn history_fills_the_null_slots_and_makes_maps() {
        let all = rows(77 * 16, 30, 0.01, 50.0);
        let mut h = RadarHistory::new(CPI_SLOTS, SLOT, BIN_KM, 200, 150);
        let mut maps = 0;
        for f in 0..16 {
            // A frame's 76 symbols; its null slot (the first) is left out.
            let profiles: Vec<Vec<Iq>> = all[f * 77 + 1..(f + 1) * 77].to_vec();
            if h.push_frame(&profiles)
                && let Some(m) = h.map()
            {
                maps += 1;
                assert_eq!(m.ranges, 150);
                let (km, hz, _) = peak(&m);
                assert!(
                    (km - 30.0 * BIN_KM).abs() < 0.01 && (hz - 50.0).abs() < 1.5,
                    "{km} {hz}"
                );
                // No copies at ±10.4 Hz: the null slots are filled.
                let d = ((50.0 + 10.4 - m.doppler_start_hz) / m.doppler_step_hz).round() as usize;
                let side = (d - 2..=d + 2)
                    .map(|d| m.at(30, d))
                    .fold(f32::MIN, f32::max);
                assert!(side < m.max_snr_db - 25.0, "{side} vs {}", m.max_snr_db);
            }
        }
        // Frame f (from 1) brings the slots to 77f − 1: full from frame 11,
        // and maps come every fifth frame, so one, at frame 15.
        assert_eq!(maps, 1);
        assert_eq!(h.len(), CPI_SLOTS);
    }

    #[test]
    fn range_counts_from_the_strongest_path() {
        // Everything 20 bins later: the map is the same.
        let shifted: Vec<Vec<Iq>> = rows(770, 120, 0.01, 85.0)
            .into_iter()
            .map(|r| {
                let mut s = vec![Iq::new(0.0, 0.0); 20];
                s.extend(r);
                s
            })
            .collect();
        let m = range_doppler(&shifted, SLOT, BIN_KM, 200).unwrap();
        assert_eq!(m.ranges, 200);
        let (km, hz, _) = peak(&m);
        assert!(
            (km - 120.0 * BIN_KM).abs() < 0.01 && (hz - 85.0).abs() < 1.5,
            "{km} {hz}"
        );
    }

    #[test]
    fn echoes_are_found_off_the_ridge() {
        let m = range_doppler(&rows(770, 120, 0.01, 85.0), SLOT, BIN_KM, 256).unwrap();
        let e = echoes(&m, 15.0, 100.0, 25.0, 5);
        assert!(!e.is_empty());
        assert!(
            (e[0].range_km - 120.0 * BIN_KM).abs() < 0.01 && (e[0].doppler_hz - 85.0).abs() < 1.5
        );
        // Beyond the range asked for: none.
        assert!(echoes(&m, 15.0, 10.0, 25.0, 5).is_empty());
    }

    #[test]
    fn too_little() {
        assert!(range_doppler(&[], SLOT, BIN_KM, 100).is_none());
        assert!(range_doppler(&rows(10, 5, 0.1, 0.0), SLOT, BIN_KM, 100).is_none());
    }
}
