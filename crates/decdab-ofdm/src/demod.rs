// made by CasualArclamp
//! The OFDM demodulator: from 2.048 MS/s samples to the soft bits of each
//! transmission frame's 75 data symbols, frequency-deinterleaved.
//!
//! **Acquisition.** The null symbol is found as the quietest 2656 samples
//! of a frame's worth; the fractional frequency offset comes from the
//! cyclic prefixes of the frame's 76 symbols, the whole-carrier offset from
//! the phase reference symbol (a differential correlation, which a timing
//! error cannot spoil).
//!
//! **Tracking.** Each frame the channel impulse response is read off the
//! phase reference (its FFT times the known symbol's conjugate, back
//! through an inverse FFT); the first path within 12 dB of the strongest is
//! kept [`ADVANCE`] samples into the guard interval, by whole samples, at
//! frame boundaries. A residual frequency offset turns every carrier by the
//! same angle from one symbol to the next: it is measured on the DQPSK
//! decisions (the fourth-power method) and taken out per symbol, and its
//! frame mean steers the NCO. A sample clock error only drifts the timing,
//! which the impulse response follows.
//!
//! Nothing here allocates per frame: the buffers are made once.
//!
//! CPU: 77 FFTs of 2048 points a frame (96 ms), about 800 a second.

use std::sync::Arc;

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

use crate::params::{
    BITS_PER_SYMBOL, CARRIERS, DATA_SYMBOLS, FFT, FRAME, GUARD, NULL, SAMPLE_RATE, SYMBOL, SYMBOLS,
    bin,
};
use crate::tables::{carrier, prs_bins};

type Iq = Complex32;

/// Samples into the guard interval the first path is kept at: room for the
/// impulse response to wander, and for paths a little earlier than the one
/// taken as first.
pub const ADVANCE: usize = 48;
/// Whole carriers either side searched for the frequency offset at
/// acquisition (±40 kHz: 170 ppm at 230 MHz).
const MAX_CARRIER_OFFSET: i32 = 40;
/// The impulse response's peak over its mean power below which a frame's
/// phase reference counts as not found.
const MIN_PEAK: f32 = 12.0;
/// Frames in a row without a phase reference before the lock is given up.
const LOSE_AFTER: u32 = 4;
/// The soft bit a carrier of average strength gives (on the Viterbi's ±127).
const SOFT_SCALE: f32 = 40.0;

/// What one frame looked like.
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    /// The frequency correction applied, Hz (the signal sits this far above
    /// the tuning).
    pub cfo_hz: f64,
    /// Whole samples the timing moved after this frame.
    pub timing_step: i32,
    /// The impulse response's peak over its mean power, dB.
    pub peak_db: f32,
    /// MER of the differential (DQPSK) decisions, dB.
    pub mer_db: f32,
    /// Frames demodulated since the lock was taken.
    pub frame: u64,
}

/// A demodulated transmission frame.
pub struct Frame<'a> {
    /// Soft bits of the 75 data symbols, 3072 each, in the order of
    /// EN 300 401 §14.5: symbol by symbol, the QPSK symbols' real parts
    /// (n = 0…1535) then their imaginary parts. Positive means 0.
    pub soft: &'a [i8],
    /// Each active carrier's equalised-differential value of the last data
    /// symbol, for the scopes (k = −768…−1, 1…768).
    pub last_diff: &'a [Iq],
    pub stats: FrameStats,
}

enum State {
    Search,
    Locked { start: usize, misses: u32 },
}

pub struct Demod {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    scratch: Vec<Iq>,
    /// Samples, already turned back by the NCO; `buf[0]` is sample `base`.
    buf: Vec<Iq>,
    state: State,
    /// NCO frequency (Hz) and phase (turns).
    cfo: f64,
    phase: f64,
    work: Vec<Iq>,
    cur: Vec<Iq>,
    prev: Vec<Iq>,
    diff: Vec<Iq>,
    soft: Vec<i8>,
    /// For each active carrier index (k = −768…−1, 1…768 → 0…1535), the
    /// QPSK symbol n it carries.
    n_of: Vec<u16>,
    frames: u64,
}

/// Index of carrier k among the active ones (−768 → 0, 768 → 1535).
#[inline]
fn slot(k: i32) -> usize {
    if k < 0 {
        (k + 768) as usize
    } else {
        (k + 767) as usize
    }
}

/// Carrier of active index i, the inverse of [`slot`].
#[inline]
fn carrier_of(i: usize) -> i32 {
    if i < 768 { i as i32 - 768 } else { i as i32 - 767 }
}

impl Default for Demod {
    fn default() -> Self {
        Self::new()
    }
}

impl Demod {
    pub fn new() -> Demod {
        let mut p = FftPlanner::new();
        let fft = p.plan_fft_forward(FFT);
        let ifft = p.plan_fft_inverse(FFT);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());
        let mut n_of = vec![0u16; CARRIERS];
        for (n, &k) in carrier().iter().enumerate() {
            n_of[slot(k as i32)] = n as u16;
        }
        Demod {
            fft,
            ifft,
            scratch: vec![Iq::default(); scratch_len],
            buf: Vec::with_capacity(3 * FRAME),
            state: State::Search,
            cfo: 0.0,
            phase: 0.0,
            work: vec![Iq::default(); FFT],
            cur: vec![Iq::default(); CARRIERS],
            prev: vec![Iq::default(); CARRIERS],
            diff: vec![Iq::default(); CARRIERS],
            soft: vec![0; DATA_SYMBOLS * BITS_PER_SYMBOL],
            n_of,
            frames: 0,
        }
    }

    pub fn is_locked(&self) -> bool {
        matches!(self.state, State::Locked { .. })
    }

    /// The frequency correction, Hz.
    pub fn cfo_hz(&self) -> f64 {
        self.cfo
    }

    /// Forget the lock (after a retune).
    pub fn reset(&mut self) {
        self.buf.clear();
        self.state = State::Search;
        self.cfo = 0.0;
        self.phase = 0.0;
    }

    /// Take samples at 2.048 MS/s; `on_frame` is called for each frame
    /// demodulated.
    pub fn push(&mut self, samples: &[Iq], mut on_frame: impl FnMut(&Frame)) {
        // The NCO: e^{−j2π·cfo·n/fs}, a phasor stepped and renormalised.
        let step = -self.cfo / SAMPLE_RATE;
        let mut ph = Iq::from_polar(1.0, (std::f64::consts::TAU * self.phase) as f32);
        let rot = Iq::from_polar(1.0, (std::f64::consts::TAU * step) as f32);
        for (i, s) in samples.iter().enumerate() {
            self.buf.push(s * ph);
            ph *= rot;
            if i % 1024 == 1023 {
                ph /= ph.norm();
            }
        }
        self.phase = (self.phase + step * samples.len() as f64).rem_euclid(1.0);

        loop {
            match self.state {
                State::Search => {
                    if !self.acquire() {
                        return;
                    }
                }
                State::Locked { start, misses } => {
                    // The frame, plus room for the timing to move late.
                    if self.buf.len() < start + FRAME + SYMBOL {
                        return;
                    }
                    self.frame(start, misses, &mut on_frame);
                }
            }
        }
    }

    /// Change the NCO by `delta` Hz from now on.
    fn steer(&mut self, delta: f64) {
        self.cfo += delta;
    }

    /// Look for a frame in the buffer. Returns false when more samples are
    /// needed.
    fn acquire(&mut self) -> bool {
        let need = 2 * FRAME + NULL;
        if self.buf.len() < need {
            return false;
        }
        // The quietest NULL samples among the first frame's starts.
        let mut cum = Vec::with_capacity(FRAME + NULL + 1);
        cum.push(0f64);
        let mut acc = 0f64;
        for s in &self.buf[..FRAME + NULL] {
            acc += s.norm_sqr() as f64;
            cum.push(acc);
        }
        let mean = acc / (FRAME + NULL) as f64 * NULL as f64;
        let (mut best, mut best_e) = (0, f64::MAX);
        for i in 0..FRAME {
            let e = cum[i + NULL] - cum[i];
            if e < best_e {
                best_e = e;
                best = i;
            }
        }
        if best_e > 0.5 * mean {
            // No null deep enough: not DAB, or not yet.
            self.buf.drain(..FRAME);
            return true;
        }
        let start = best;
        // The fractional offset from the guard intervals: x[n]·x*[n+N]
        // turns by −2π·f·N/fs.
        let mut c = Iq::default();
        for l in 0..SYMBOLS {
            let s = start + NULL + l * SYMBOL;
            for n in s + GUARD / 4..s + GUARD - GUARD / 8 {
                c += self.buf[n] * self.buf[n + FFT].conj();
            }
        }
        let frac = -(c.arg() as f64) / std::f64::consts::TAU * (SAMPLE_RATE / FFT as f64);
        // Turn the buffer back by that much, then find the whole carriers
        // on the phase reference.
        self.retune_buffer(frac);
        let w = start + NULL + GUARD - GUARD / 2;
        self.spectrum(w);
        let p = prs_bins();
        let mut best_d = (0, 0f32, 0f32);
        for d in -MAX_CARRIER_OFFSET..=MAX_CARRIER_OFFSET {
            let mut acc = Iq::default();
            let mut prev: Option<Iq> = None;
            for k in -768..=768 {
                if k == 0 {
                    prev = None;
                    continue;
                }
                let z = self.work[bin(k + d)] * p[bin(k)].conj();
                if let Some(q) = prev {
                    acc += z * q.conj();
                }
                prev = Some(z);
            }
            let m = acc.norm();
            best_d.2 += m;
            if m > best_d.1 {
                best_d = (d, m, best_d.2);
            }
        }
        let mean_m = best_d.2 / (2 * MAX_CARRIER_OFFSET + 1) as f32;
        if best_d.1 < 4.0 * mean_m {
            self.buf.drain(..FRAME);
            return true;
        }
        let whole = best_d.0 as f64 * (SAMPLE_RATE / FFT as f64);
        self.retune_buffer(whole);
        // Fine timing comes from the first frame's impulse response.
        self.state = State::Locked {
            start: start.saturating_sub(GUARD / 2),
            misses: 0,
        };
        self.frames = 0;
        true
    }

    /// Apply a further `delta` Hz of correction to the samples buffered and
    /// to those still to come.
    fn retune_buffer(&mut self, delta: f64) {
        let rot = Iq::from_polar(1.0, (-std::f64::consts::TAU * delta / SAMPLE_RATE) as f32);
        let mut ph = Iq::new(1.0, 0.0);
        for (i, s) in self.buf.iter_mut().enumerate() {
            *s *= ph;
            ph *= rot;
            if i % 1024 == 1023 {
                ph /= ph.norm();
            }
        }
        // Keep the NCO's phase continuous with the last sample turned.
        self.phase = (self.phase - delta / SAMPLE_RATE * self.buf.len() as f64).rem_euclid(1.0);
        self.steer(delta);
    }

    /// FFT of the 2048 samples from `at` into `self.work`.
    fn spectrum(&mut self, at: usize) {
        self.work.copy_from_slice(&self.buf[at..at + FFT]);
        self.fft
            .process_with_scratch(&mut self.work, &mut self.scratch);
    }

    /// The active carriers of the FFT in `self.work`, into `out`.
    fn carriers(work: &[Iq], out: &mut [Iq]) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = work[bin(carrier_of(i))];
        }
    }

    fn frame(&mut self, start: usize, misses: u32, on_frame: &mut impl FnMut(&Frame)) {
        let sym = |l: usize| start + NULL + l * SYMBOL + GUARD - ADVANCE;
        // The phase reference, and from it the impulse response.
        self.spectrum(sym(0));
        Self::carriers(&self.work, &mut self.prev);
        let p = prs_bins();
        for (w, q) in self.work.iter_mut().zip(p.iter()) {
            *w *= q.conj();
        }
        self.ifft
            .process_with_scratch(&mut self.work, &mut self.scratch);
        let pow: Vec<f32> = self.work.iter().map(|z| z.norm_sqr()).collect();
        let mean = pow.iter().sum::<f32>() / FFT as f32;
        let (peak_i, peak) = pow
            .iter()
            .enumerate()
            .fold((0, 0f32), |b, (i, &v)| if v > b.1 { (i, v) } else { b });
        let peak_ratio = peak / mean.max(1e-30);
        let peak_db = 10.0 * peak_ratio.log10();
        // Signed delays: the window is early by `d` when a path shows at d.
        let signed = |i: usize| -> i32 {
            if i < FFT / 2 {
                i as i32
            } else {
                i as i32 - FFT as i32
            }
        };
        let found = peak_ratio >= MIN_PEAK;
        let mut step = 0i32;
        if found {
            // The first path within 12 dB of the strongest, searching back
            // from the strongest by up to the guard interval.
            let pk = signed(peak_i);
            let mut first = pk;
            let thr = (peak / 16.0).max(10.0 * mean);
            for d in (pk - GUARD as i32)..pk {
                if pow[d.rem_euclid(FFT as i32) as usize] > thr {
                    first = d;
                    break;
                }
            }
            // Wanted at ADVANCE; move by whole samples, at most a guard's
            // worth at a time.
            step = (first - ADVANCE as i32).clamp(-(GUARD as i32), GUARD as i32);
        }

        // The data symbols.
        let mut cpe_sum = 0f64;
        let (mut err_acc, mut sig_acc) = (0f64, 0f64);
        for l in 1..SYMBOLS {
            self.spectrum(sym(l));
            Self::carriers(&self.work, &mut self.cur);
            let mut q = Iq::default();
            let mut amp = 0f32;
            for ((d, c), p) in self.diff.iter_mut().zip(&self.cur).zip(&self.prev) {
                *d = c * p.conj();
                let d2 = *d * *d;
                q += d2 * d2 / d.norm().max(1e-30).powi(3);
                amp += c.norm();
            }
            // QPSK at ±45°, ±135°: d⁴ points at π. The residual turn per
            // symbol is the rest, a quarter of it.
            let cpe = ((-q).arg()) / 4.0;
            cpe_sum += cpe as f64;
            let derot = Iq::from_polar(1.0, -cpe);
            let scale = SOFT_SCALE * CARRIERS as f32 / amp.max(1e-30);
            let base = (l - 1) * BITS_PER_SYMBOL;
            for (i, (d, p)) in self.diff.iter_mut().zip(&self.prev).enumerate() {
                // d/|prev|: the new carrier's amplitude, the phase step.
                let z = *d * derot / p.norm().max(1e-30) * scale;
                *d = z;
                let n = self.n_of[i] as usize;
                self.soft[base + n] = z.re.clamp(-127.0, 127.0) as i8;
                self.soft[base + CARRIERS + n] = z.im.clamp(-127.0, 127.0) as i8;
                // MER against the nearest QPSK point.
                let a = z.norm() * std::f32::consts::FRAC_1_SQRT_2;
                let ideal = Iq::new(a.copysign(z.re), a.copysign(z.im));
                err_acc += (z - ideal).norm_sqr() as f64;
                sig_acc += ideal.norm_sqr() as f64;
            }
            std::mem::swap(&mut self.cur, &mut self.prev);
        }
        // The residual offset: the mean turn per symbol.
        let resid_hz = cpe_sum / (SYMBOLS - 1) as f64 / std::f64::consts::TAU
            * (SAMPLE_RATE / SYMBOL as f64);
        let mer_db = (10.0 * (sig_acc / err_acc.max(1e-30)).log10()) as f32;

        let stats = FrameStats {
            cfo_hz: self.cfo,
            timing_step: step,
            peak_db,
            mer_db,
            frame: self.frames,
        };
        if found {
            self.frames += 1;
            on_frame(&Frame {
                soft: &self.soft,
                last_diff: &self.diff,
                stats,
            });
            // Steer half the residual: the samples already buffered keep the
            // old correction, which the per-symbol turn absorbs.
            self.steer(0.5 * resid_hz);
        }
        let misses = if found { 0 } else { misses + 1 };
        let next = (start as i64 + FRAME as i64 + step as i64).max(0) as usize;
        // Drop what is behind the next frame (keeping a guard's room).
        let keep_from = next.saturating_sub(GUARD);
        self.buf.drain(..keep_from);
        self.state = if misses >= LOSE_AFTER {
            State::Search
        } else {
            State::Locked {
                start: next - keep_from,
                misses,
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_round_trip() {
        for i in 0..CARRIERS {
            assert_eq!(slot(carrier_of(i)), i);
        }
        assert_eq!(carrier_of(0), -768);
        assert_eq!(carrier_of(767), -1);
        assert_eq!(carrier_of(768), 1);
        assert_eq!(carrier_of(1535), 768);
    }
}
