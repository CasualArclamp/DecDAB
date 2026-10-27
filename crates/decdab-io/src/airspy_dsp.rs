// made by CasualArclamp
//! The Airspy R2's samples to complex baseband at a rate the engine takes.
//!
//! The R2's bulk stream is a **real** 12-bit signal at 20 MS/s: the R820T
//! tuner puts the wanted channel on a 5 MHz IF, which is a quarter of the
//! sample rate. libairspy turns that into 10 MS/s IQ with its `iqconverter`:
//! remove DC, multiply by e^(jπn/2) (at fs/4 that is only sign changes and a
//! swap of every other sample into Q), then a 31-tap half-band low-pass that
//! keeps every second output. Here the same maths is rearranged to be cheap
//! and then carried on down to DAB's rate:
//!
//! 1. **Load**: 12-bit offset-binary to f32, the DC offset taken off, and the
//!    fs/4 translation's signs applied. Even samples become I, odd ones Q.
//! 2. **libairspy's half-band, decimating by two.** Its non-zero taps sit at
//!    even positions, so the filter only ever touches the even samples
//!    (a 16-tap symmetric FIR on I); Q is the centre tap alone: the odd
//!    samples delayed by 8 and scaled by 0.5. The Q path costs nothing.
//! 3. **Further half-band decimators** (zero or more) at 10 MS/s, 5 MS/s…
//!    until the rate is not far above the output.
//! 4. **A polyphase rational resampler** to 2.048, 4.096 or 8.192 MS/s.
//!    DAB needs ±0.77 MHz, not the converter's ±3.9 MHz, so the anti-alias
//!    filter is narrow and short.
//!
//! Everything works on a few thousand samples at a time in pre-sized buffers
//! (so nothing is allocated per block and the working set stays in L1/L2),
//! with the history each filter needs carried at the front of its buffer.
//! The inner loops are written as element-wise operations over contiguous
//! slices so LLVM turns them into SSE/AVX.
//!
//! The half-band's taps and the fs/4 sign pattern are libairspy's
//! (`iqconverter_float.c`, `filters.h`; airspyone_host, BSD-3-Clause).
//!
//! # Spectral orientation — needs a live check
//!
//! The sign pattern decides whether the band comes out mirrored. libairspy's
//! assumes the R820T presents an RF signal at `centre + δ` on the IF at
//! `5 MHz − δ` (the tuner mixes with its LO above the signal, which inverts
//! the spectrum), and the fs/4 translation, an upward shift of 5 MHz,
//! turns that back to `+δ`. This cannot be checked without a radio; if the
//! live band comes out mirrored, flip [`MIRROR_SPECTRUM`] (the only place the
//! orientation is decided). The tests derive their expectation from it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use decdab_core::{Error, Iq, Result};
use decdab_dsp::resample::{kaiser_lowpass, rational_ratio};

use crate::airspy_proto::unpack8;

/// **The spectral orientation.** `false`: libairspy's, which assumes the
/// IF is inverted (RF `centre + δ` at `5 MHz − δ`) and so puts RF
/// `centre + δ` at `+δ` in the output. `true`: the complex conjugate (Q
/// negated), for a radio whose band turns out mirrored.
pub const MIRROR_SPECTRUM: bool = false;

/// libairspy's half-band (`hb_kernel_float`): the 16 non-zero taps of a
/// 31-tap, equiripple, symmetric half-band at 20 MS/s — h[0], h[2], … h[30]
/// — flat to 3.9 MHz within 0.013 dB peak to peak, with 58.6 dB of rejection from
/// 6.1 MHz (measured by `libairspys_halfband_is_what_the_spec_says`; libairspy's
/// own figure is a little better). The centre tap h[15] is [`HB_CENTRE`]; the others (odd distance
/// from it) are zero.
// The digits are libairspy's, kept as printed there.
#[allow(clippy::excessive_precision)]
pub const HB: [f32; 16] = [
    -0.001_471_609,
    0.003_776_939,
    -0.008_221_535,
    0.015_790_741,
    -0.028_336_383,
    0.050_286_030,
    -0.097_619_457,
    0.315_607_795,
    0.315_607_795,
    -0.097_619_457,
    0.050_286_030,
    -0.028_336_383,
    0.015_790_741,
    -0.008_221_535,
    0.003_776_939,
    -0.001_471_609,
];
/// The half-band's centre tap h[15] (libairspy's `hbc`).
pub const HB_CENTRE: f32 = 0.5;

/// What a full-scale ADC tone comes out as. A real tone of amplitude `a`
/// (a fraction of full scale) is two complex tones of `a/2`; this makes up
/// the half, so a full-scale tone gives |IQ| = 1.0, the same as a full-scale
/// complex tone from the HackRF's cs8 path (`bytes_to_iq`: ±1.0) and the
/// engine's clipping figure (0.99) means the same on both.
pub const OUTPUT_GAIN: f32 = 2.0;

/// The ADC's code at zero volts and the scale of one code, both from
/// libairspy (`x = (raw − 2048) / 2048`).
const ADC_MID: f32 = 2048.0;
const ADC_SCALE: f32 = 1.0 / 2048.0;
/// How fast the DC estimate follows the block means (per chunk of 8192
/// samples: a time constant of ~100 ms). The offset would otherwise end up
/// at 5 MHz after the translation, where the later filters take it out, so
/// this only keeps the numbers centred; libairspy's per-sample recursion
/// (`avg += 0.01·x`) is a serial dependency the vector units cannot use.
const DC_ALPHA: f32 = 1.0 / 256.0;

/// Wanted half-bandwidth: DAB's carriers reach ±0.768 MHz. The anti-alias
/// filters keep ±0.8 MHz flat.
const PASS_HZ: f64 = 0.8e6;
/// Stop-band attenuation the half-bands and the resampler are designed for,
/// dB (the target is better than 50).
const HALFBAND_ATTEN_DB: f64 = 60.0;
const RESAMPLER_ATTEN_DB: f64 = 65.0;

/// Complex samples at 10 MS/s per chunk: 8192 raw samples, 16 KiB.
const CHUNK_M: usize = 4096;
/// History the load stage keeps: I needs 15 earlier samples, Q one 8 back.
const E_HIST: usize = 15;
const O_HIST: usize = 8;
/// The stage-1 sums are done this many outputs at a time, in a stack array.
const BLOCK: usize = 256;

/// The output rates the engine takes.
pub const OUTPUT_RATES_HZ: [u32; 3] = [2_048_000, 4_096_000, 8_192_000];

/// How many half-band decimators to put before the resampler. Measured, not
/// guessed (`bench_converter`): each one halves the resampler's input rate
/// and with it the taps it needs per output, which pays for itself at 4.096
/// MS/s (one: 5.5 % of a core against 6.5 % with none) and at 2.048 (two:
/// 6.9 % against 9.0 % with one). At 8.192 none is worth it.
pub fn default_halfbands(out_rate_hz: u32) -> usize {
    if out_rate_hz <= 2_048_000 {
        2
    } else if out_rate_hz <= 4_096_000 {
        1
    } else {
        0
    }
}

/// A stage's input: two planes (I and Q) with `hist` samples of history in
/// front of the new ones.
struct Planes {
    i: Vec<f32>,
    q: Vec<f32>,
    hist: usize,
}

impl Planes {
    fn new(hist: usize, cap: usize) -> Planes {
        Planes {
            i: vec![0.0; hist + cap],
            q: vec![0.0; hist + cap],
            hist,
        }
    }

    /// The room for `n` new samples.
    fn fresh(&mut self, n: usize) -> (&mut [f32], &mut [f32]) {
        (
            &mut self.i[self.hist..self.hist + n],
            &mut self.q[self.hist..self.hist + n],
        )
    }

    /// `n` samples have been consumed: keep the last `hist` of everything for
    /// the next block.
    fn roll(&mut self, n: usize) {
        let h = self.hist;
        self.i.copy_within(n..n + h, 0);
        self.q.copy_within(n..n + h, 0);
    }

    fn clear(&mut self) {
        self.i.fill(0.0);
        self.q.fill(0.0);
    }
}

/// A half-band low-pass that keeps every second output. Its taps at even
/// distances from the (odd-numbered) centre are zero, so only the odd ones
/// are stored, and the input is split into its even and odd samples first:
/// the centre tap and the rest then each read one contiguous plane, which is
/// what lets the compiler vectorise across outputs.
struct Halfband {
    /// Taps at distance 1, 3, 5, … from the centre (symmetric).
    taps: Vec<f32>,
    /// Index of the centre tap.
    c: usize,
    /// The even and odd input samples.
    even: Vec<f32>,
    odd: Vec<f32>,
}

impl Halfband {
    /// For an input at `fs_in` Hz, at most `cap` new samples a call: wanted
    /// ±[`PASS_HZ`], the stop-band from where decimating by two folds it onto
    /// the wanted band.
    fn new(fs_in: f64, cap: usize) -> Halfband {
        let a = HALFBAND_ATTEN_DB;
        let width = (fs_in / 2.0 - 2.0 * PASS_HZ) / fs_in;
        let want = ((a - 7.95) / (14.36 * width)).ceil() as usize + 1;
        // A half-band has 4k + 3 taps: odd, with the end taps non-zero.
        let mut n = want.max(7);
        while n % 4 != 3 {
            n += 1;
        }
        let h = kaiser_lowpass(n, 0.25, 0.1102 * (a - 8.7));
        let c = (n - 1) / 2;
        let mut taps: Vec<f64> = (0..n.div_ceil(4))
            .map(|i| f64::from(h[c + 2 * i + 1]))
            .collect();
        // DC gain exactly one: the centre is 0.5, so the pairs sum to 0.25.
        let s: f64 = taps.iter().sum();
        for t in &mut taps {
            *t *= 0.25 / s;
        }
        Halfband {
            taps: taps.into_iter().map(|t| t as f32).collect(),
            c,
            even: vec![0.0; (2 * c + cap) / 2],
            odd: vec![0.0; (2 * c + cap) / 2],
        }
    }

    fn len(&self) -> usize {
        2 * self.c + 1
    }

    /// `n_in` (even) new samples, with `len() − 1` of history in front, to
    /// `n_in / 2` outputs, those at the even positions:
    /// `y[p] = ½·x[2p + c] + Σ_i t_i (x[2p + c − d] + x[2p + c + d])`, `d` =
    /// 2i + 1. With `c = 2h + 1` that reads the odd samples at `p + h` for the
    /// centre and the even ones at `p + h − i` and `p + h + 1 + i`.
    fn decimate(&mut self, x: &[f32], n_in: usize, out: &mut [f32]) {
        let half = (2 * self.c + n_in) / 2;
        for ((e, o), pair) in self.even[..half]
            .iter_mut()
            .zip(&mut self.odd[..half])
            .zip(x.as_chunks::<2>().0)
        {
            *e = pair[0];
            *o = pair[1];
        }
        let h = (self.c - 1) / 2;
        let n_out = n_in / 2;
        let mut b = 0;
        while b < n_out {
            let len = BLOCK.min(n_out - b);
            let mut acc = [0f32; BLOCK];
            for (a, &c) in acc[..len].iter_mut().zip(&self.odd[b + h..b + h + len]) {
                *a = HB_CENTRE * c;
            }
            for (i, &t) in self.taps.iter().enumerate() {
                let lo = &self.even[b + h - i..b + h - i + len];
                let hi = &self.even[b + h + 1 + i..b + h + 1 + i + len];
                for ((a, &l), &u) in acc[..len].iter_mut().zip(lo).zip(hi) {
                    *a += t * (l + u);
                }
            }
            out[b..b + len].copy_from_slice(&acc[..len]);
            b += len;
        }
    }
}

/// A polyphase rational resampler on two planes.
struct PolyResampler {
    up: usize,
    /// `down / up` = `q + r / up`: how far the input position moves for each
    /// output.
    q: usize,
    r: usize,
    /// Taps per phase, a multiple of four.
    t: usize,
    /// `rows[φ·t ..][..t]`: phase φ's taps, oldest input first.
    rows: Vec<f32>,
    /// Position of the next output: the newest input it uses (counted in the
    /// block to come) and the phase within that sample's interval.
    n: usize,
    phi: usize,
}

impl PolyResampler {
    fn new(in_hz: f64, out_hz: f64) -> Result<PolyResampler> {
        let (up, down) = rational_ratio(in_hz, out_hz, 2048)
            .ok_or_else(|| Error::Other(format!("Airspy: no ratio from {in_hz} to {out_hz} Hz")))?;
        // The wanted band must pass; what folds onto it (from `out − pass`
        // up) must go.
        let stop = out_hz.min(in_hz) - PASS_HZ;
        if stop <= PASS_HZ * 1.05 {
            return Err(Error::Other(format!(
                "Airspy: a {:.2} MHz output cannot hold a DAB ensemble",
                out_hz / 1e6
            )));
        }
        let f = in_hz * up as f64;
        let a = RESAMPLER_ATTEN_DB;
        let width = (stop - PASS_HZ) / f;
        let len = ((a - 7.95) / (14.36 * width)).ceil() as usize + 1;
        let t = len.div_ceil(up).div_ceil(4) * 4;
        // A symmetric prototype of odd length, one zero tap to fill the last
        // phase, and the interpolation's gain of `up`.
        let mut h = kaiser_lowpass(t * up - 1, (PASS_HZ + stop) / 2.0 / f, 0.1102 * (a - 8.7));
        h.push(0.0);
        let mut rows = vec![0.0; up * t];
        for phi in 0..up {
            for k in 0..t {
                // Input `k` samples old has weight h[φ + k·up]; stored with
                // the oldest first so it lines up with the buffer.
                rows[phi * t + (t - 1 - k)] = h[phi + k * up] * up as f32;
            }
        }
        Ok(PolyResampler {
            up,
            q: down / up,
            r: down % up,
            t,
            rows,
            n: 0,
            phi: 0,
        })
    }

    /// `n_in` new samples (with `t − 1` of history in front) to outputs,
    /// returning how many.
    fn process(&mut self, x: &Planes, n_in: usize, oi: &mut [f32], oq: &mut [f32]) -> usize {
        let t = self.t;
        let (mut n, mut phi, mut count) = (self.n, self.phi, 0);
        while n < n_in {
            let row = &self.rows[phi * t..(phi + 1) * t];
            (oi[count], oq[count]) = dot2(row, &x.i[n..n + t], &x.q[n..n + t]);
            count += 1;
            phi += self.r;
            n += self.q;
            if phi >= self.up {
                phi -= self.up;
                n += 1;
            }
        }
        self.n = n - n_in;
        self.phi = phi;
        count
    }
}

/// Two real dot products against the same taps. Four partial sums each:
/// the compiler will not reorder float additions, so a single running sum
/// would be a serial chain and these four lanes become one vector.
#[inline]
fn dot2(row: &[f32], xi: &[f32], xq: &[f32]) -> (f32, f32) {
    let (mut ai, mut aq) = ([0f32; 4], [0f32; 4]);
    for ((c, i), q) in row
        .as_chunks::<4>()
        .0
        .iter()
        .zip(xi.as_chunks::<4>().0)
        .zip(xq.as_chunks::<4>().0)
    {
        for l in 0..4 {
            ai[l] += c[l] * i[l];
            aq[l] += c[l] * q[l];
        }
    }
    (
        (ai[0] + ai[2]) + (ai[1] + ai[3]),
        (aq[0] + aq[2]) + (aq[1] + aq[3]),
    )
}

/// The converter: raw Airspy bytes in, IQ at the output rate out.
pub struct IqConverter {
    packed: bool,
    out_hz: u32,
    /// The DC estimate, in ADC codes.
    dc: f32,
    /// The load stage's output with history: I-path (even) and Q-path (odd)
    /// samples.
    e: Vec<f32>,
    o: Vec<f32>,
    halfbands: Vec<Halfband>,
    /// The input of each half-band, then of the resampler: `halfbands.len()
    /// + 1` of them.
    links: Vec<Planes>,
    resampler: PolyResampler,
    out_i: Vec<f32>,
    out_q: Vec<f32>,
    /// A group of samples split across two calls.
    carry: [u8; 16],
    carry_len: usize,
}

impl IqConverter {
    /// A converter to `out_rate_hz` (2.048, 4.096 or 8.192 MS/s) with the
    /// default number of half-bands. `packed`: the input is 12-bit packed.
    pub fn new(out_rate_hz: u32, packed: bool) -> Result<Self> {
        Self::with_halfbands(out_rate_hz, packed, default_halfbands(out_rate_hz))
    }

    pub fn with_halfbands(out_rate_hz: u32, packed: bool, halfbands: usize) -> Result<Self> {
        if !OUTPUT_RATES_HZ.contains(&out_rate_hz) {
            return Err(Error::Other(format!(
                "Airspy: output rate {out_rate_hz} Hz is not one of 2.048, 4.096, 8.192 MS/s"
            )));
        }
        let mut fs = f64::from(crate::airspy_proto::IQ_RATE_HZ);
        let mut stages = Vec::new();
        let mut cap = CHUNK_M;
        for _ in 0..halfbands {
            stages.push(Halfband::new(fs, cap));
            fs /= 2.0;
            cap /= 2;
        }
        if fs < f64::from(out_rate_hz) * 1.1 {
            return Err(Error::Other(
                "Airspy: too many half-bands for this output rate".into(),
            ));
        }
        let resampler = PolyResampler::new(fs, f64::from(out_rate_hz))?;
        // Every stage but the first gets `CHUNK_M / 2^k` samples a block.
        let mut links = Vec::new();
        let mut cap = CHUNK_M;
        for s in &stages {
            links.push(Planes::new(s.len() - 1, cap));
            cap /= 2;
        }
        links.push(Planes::new(resampler.t - 1, cap));
        Ok(IqConverter {
            packed,
            out_hz: out_rate_hz,
            dc: ADC_MID,
            e: vec![0.0; E_HIST + CHUNK_M],
            o: vec![0.0; O_HIST + CHUNK_M],
            halfbands: stages,
            links,
            resampler,
            out_i: vec![0.0; CHUNK_M + 8],
            out_q: vec![0.0; CHUNK_M + 8],
            carry: [0; 16],
            carry_len: 0,
        })
    }

    pub fn output_rate(&self) -> f64 {
        f64::from(self.out_hz)
    }

    /// Back to the state of a new converter: after a retune, so the old
    /// channel's samples are not smeared into the new one.
    pub fn reset(&mut self) {
        self.dc = ADC_MID;
        self.e.fill(0.0);
        self.o.fill(0.0);
        for l in &mut self.links {
            l.clear();
        }
        self.resampler.n = 0;
        self.resampler.phi = 0;
        self.carry_len = 0;
    }

    /// Bytes per group the converter works in (4 complex samples).
    fn unit(&self) -> usize {
        if self.packed { 12 } else { 16 }
    }

    /// Convert `bytes` from the radio, appending the IQ to `out`. Any length:
    /// what does not fill a group of 8 samples is kept for the next call.
    pub fn process(&mut self, bytes: &[u8], out: &mut Vec<Iq>) {
        let unit = self.unit();
        let mut bytes = bytes;
        if self.carry_len > 0 {
            let take = (unit - self.carry_len).min(bytes.len());
            self.carry[self.carry_len..self.carry_len + take].copy_from_slice(&bytes[..take]);
            self.carry_len += take;
            bytes = &bytes[take..];
            if self.carry_len < unit {
                return;
            }
            let group = self.carry;
            self.carry_len = 0;
            self.run(&group[..unit], out);
        }
        let whole = bytes.len() / unit * unit;
        self.run(&bytes[..whole], out);
        let rest = &bytes[whole..];
        self.carry[..rest.len()].copy_from_slice(rest);
        self.carry_len = rest.len();
    }

    /// Whole groups only.
    fn run(&mut self, bytes: &[u8], out: &mut Vec<Iq>) {
        // Four complex samples (one group) are `unit` bytes: bytes a sample
        // pair is `unit / 4`.
        let per_pair = self.unit() / 4;
        let pairs = (bytes.len() / per_pair) as u64;
        out.reserve(
            (pairs * u64::from(self.out_hz) / u64::from(crate::airspy_proto::IQ_RATE_HZ)) as usize
                + 8,
        );
        for chunk in bytes.chunks(CHUNK_M * per_pair) {
            self.chunk(chunk, chunk.len() / per_pair, out);
        }
    }

    /// `m` (a multiple of four) complex samples at 10 MS/s' worth of input.
    fn chunk(&mut self, bytes: &[u8], m: usize, out: &mut Vec<Iq>) {
        // 1. Load.
        let (e, o) = (
            &mut self.e[E_HIST..E_HIST + m],
            &mut self.o[O_HIST..O_HIST + m],
        );
        let sum = if self.packed {
            load_packed(bytes, self.dc, e, o)
        } else {
            load_unpacked(bytes, self.dc, e, o)
        };
        self.dc += (sum as f32 / (2 * m) as f32 - self.dc) * DC_ALPHA;

        // 2. libairspy's half-band, to 10 MS/s.
        {
            let (wi, wq) = self.links[0].fresh(m);
            stage1(&self.e, &self.o, m, wi, wq);
        }
        self.e.copy_within(m..m + E_HIST, 0);
        self.o.copy_within(m..m + O_HIST, 0);

        // 3. The half-bands.
        let mut n = m;
        for (k, hb) in self.halfbands.iter_mut().enumerate() {
            let (a, b) = self.links.split_at_mut(k + 1);
            let (src, dst) = (&mut a[k], &mut b[0]);
            let (di, dq) = dst.fresh(n / 2);
            hb.decimate(&src.i, n, di);
            hb.decimate(&src.q, n, dq);
            src.roll(n);
            n /= 2;
        }

        // 4. The resampler.
        let last = self
            .links
            .last_mut()
            .expect("at least the resampler's input");
        let count = self
            .resampler
            .process(last, n, &mut self.out_i, &mut self.out_q);
        last.roll(n);
        out.extend(
            self.out_i[..count]
                .iter()
                .zip(&self.out_q[..count])
                .map(|(&i, &q)| Iq::new(i, q)),
        );
    }
}

/// Unpacked samples: 16-bit little-endian, even ones to `e`, odd ones to `o`,
/// each scaled and carrying the fs/4 translation's sign (−, +, −, + … for
/// the `n`-th pair; see the module comment). Returns the sum of the raw
/// samples, for the DC estimate.
fn load_unpacked(bytes: &[u8], dc: f32, e: &mut [f32], o: &mut [f32]) -> u32 {
    let (neg, pos) = (-ADC_SCALE, ADC_SCALE);
    let mut sum = 0u32;
    for ((b, e), o) in bytes
        .as_chunks::<8>()
        .0
        .iter()
        .zip(e.as_chunks_mut::<2>().0)
        .zip(o.as_chunks_mut::<2>().0)
    {
        let r = [
            u16::from_le_bytes([b[0], b[1]]),
            u16::from_le_bytes([b[2], b[3]]),
            u16::from_le_bytes([b[4], b[5]]),
            u16::from_le_bytes([b[6], b[7]]),
        ];
        e[0] = (f32::from(r[0]) - dc) * neg;
        o[0] = (f32::from(r[1]) - dc) * neg;
        e[1] = (f32::from(r[2]) - dc) * pos;
        o[1] = (f32::from(r[3]) - dc) * pos;
        sum += u32::from(r[0]) + u32::from(r[1]) + u32::from(r[2]) + u32::from(r[3]);
    }
    sum
}

/// The same from 12-bit packed samples, eight to 12 bytes.
fn load_packed(bytes: &[u8], dc: f32, e: &mut [f32], o: &mut [f32]) -> u32 {
    let (neg, pos) = (-ADC_SCALE, ADC_SCALE);
    let mut sum = 0u32;
    for ((g, e), o) in bytes
        .as_chunks::<12>()
        .0
        .iter()
        .zip(e.as_chunks_mut::<4>().0)
        .zip(o.as_chunks_mut::<4>().0)
    {
        let r = unpack8(g);
        for k in 0..4 {
            let s = if k % 2 == 0 { neg } else { pos };
            e[k] = (f32::from(r[2 * k]) - dc) * s;
            o[k] = (f32::from(r[2 * k + 1]) - dc) * s;
            sum += u32::from(r[2 * k]) + u32::from(r[2 * k + 1]);
        }
    }
    sum
}

/// libairspy's half-band, evaluated where it keeps its outputs. `e` and `o`
/// hold `E_HIST`/`O_HIST` samples of history then `m` new ones; the output
/// `m`-th complex sample is
///
/// ```text
/// I = GAIN · Σ_{j=0..15} h[2j] · e[m − j]       (symmetric: pairs j and 15 − j)
/// Q = GAIN · 0.5 · o[m − 8]                     (the centre tap, h[15])
/// ```
///
/// with the signs of the translation already in `e` and `o`.
fn stage1(e: &[f32], o: &[f32], m: usize, wi: &mut [f32], wq: &mut [f32]) {
    let mut b = 0;
    while b < m {
        let len = BLOCK.min(m - b);
        let mut acc = [0f32; BLOCK];
        for j in 0..8 {
            let g = OUTPUT_GAIN * HB[j];
            // e[m − j] and e[m − 15 + j], with E_HIST = 15 in front.
            let hi = &e[b + E_HIST - j..b + E_HIST - j + len];
            let lo = &e[b + j..b + j + len];
            for ((a, &x), &y) in acc[..len].iter_mut().zip(hi).zip(lo) {
                *a += g * (x + y);
            }
        }
        wi[b..b + len].copy_from_slice(&acc[..len]);
        b += len;
    }
    let q = if MIRROR_SPECTRUM { -1.0 } else { 1.0 } * OUTPUT_GAIN * HB_CENTRE;
    // `o[m − 8]` with O_HIST = 8 in front is `o[m]` of the buffer.
    for (w, &x) in wq[..m].iter_mut().zip(&o[..m]) {
        *w = q * x;
    }
}

/// Raw buffers from the radio's reader to the converter, and back again for
/// re-use: everything between the USB thread and the engine that needs no
/// USB, so it is tested with a fake reader.
pub struct RawStream {
    rx: Receiver<Vec<u8>>,
    recycle: Sender<Vec<u8>>,
    conv: IqConverter,
    /// Buffers still to be thrown away after a retune.
    skip: u32,
    /// How long to wait for a buffer before calling the radio stalled.
    timeout: Duration,
    what: &'static str,
}

impl RawStream {
    pub fn new(
        rx: Receiver<Vec<u8>>,
        recycle: Sender<Vec<u8>>,
        conv: IqConverter,
        what: &'static str,
    ) -> Self {
        RawStream {
            rx,
            recycle,
            conv,
            skip: 0,
            timeout: Duration::from_secs(2),
            what,
        }
    }

    pub fn set_timeout(&mut self, t: Duration) {
        self.timeout = t;
    }

    pub fn sample_rate(&self) -> f64 {
        self.conv.output_rate()
    }

    /// The next block of IQ in `out` (cleared first).
    pub fn read(&mut self, out: &mut Vec<Iq>) -> Result<usize> {
        out.clear();
        loop {
            match self.rx.recv_timeout(self.timeout) {
                Ok(bytes) => {
                    if self.skip > 0 {
                        self.skip -= 1;
                    } else {
                        self.conv.process(&bytes, out);
                    }
                    // Back to the reader; if it has gone nobody needs it.
                    let _ = self.recycle.send(bytes);
                    if !out.is_empty() {
                        return Ok(out.len());
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(Error::Other(format!(
                        "{} stopped delivering samples",
                        self.what
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Other(format!(
                        "{} reader stopped (unplugged?)",
                        self.what
                    )));
                }
            }
        }
    }

    /// After a retune: throw away what is queued and the next `skip` buffers
    /// (the radio was still filling them as it retuned), and start the
    /// filters afresh.
    pub fn restart(&mut self, skip: u32) {
        while let Ok(b) = self.rx.try_recv() {
            let _ = self.recycle.send(b);
        }
        self.skip = skip;
        self.conv.reset();
    }

    /// Empty the queue (the reader is stopping and may be waiting to hand
    /// over a buffer).
    pub fn drain(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::{FftPlanner, num_complex::Complex};
    use std::f64::consts::TAU;

    const FS: f64 = 20e6;

    /// `n` raw samples of a real tone at `f_if` Hz, amplitude `amp` (a
    /// fraction of full scale), as the radio sends them: 12-bit offset-binary
    /// in 16-bit little-endian.
    fn raw_tone(f_if: f64, amp: f64, n: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(2 * n);
        for k in 0..n {
            let x = 2048.0 + 2048.0 * amp * (TAU * f_if * k as f64 / FS + 0.3).cos();
            let r = x.round().clamp(0.0, 4095.0) as u16;
            v.extend(r.to_le_bytes());
        }
        v
    }

    /// The same bytes packed 8 samples to 12.
    fn pack(raw: &[u8]) -> Vec<u8> {
        let s: Vec<u32> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u32::from(u16::from_le_bytes(*b)))
            .collect();
        let mut out = Vec::new();
        for o in s.as_chunks::<8>().0 {
            let w0 = (o[0] << 20) | (o[1] << 8) | (o[2] >> 4);
            let w1 = ((o[2] & 0xF) << 28) | (o[3] << 16) | (o[4] << 4) | (o[5] >> 8);
            let w2 = ((o[5] & 0xFF) << 24) | (o[6] << 12) | o[7];
            for w in [w0, w1, w2] {
                out.extend(w.to_le_bytes());
            }
        }
        out
    }

    fn convert(rate: u32, raw: &[u8]) -> Vec<Iq> {
        let mut c = IqConverter::new(rate, false).unwrap();
        let mut out = Vec::new();
        c.process(raw, &mut out);
        out
    }

    /// The spectrum of the last 2^15 samples of `x` (Hann-windowed): the
    /// power in each bin, normalised so a complex tone of amplitude A has
    /// total power A² across its lobe, and the bin frequencies.
    struct Spectrum {
        p: Vec<f64>,
        rate: f64,
    }

    impl Spectrum {
        const N: usize = 1 << 15;

        fn of(x: &[Iq], rate: f64) -> Spectrum {
            let n = Self::N;
            let x = &x[x.len() - n..];
            let w: Vec<f64> = (0..n)
                .map(|k| 0.5 - 0.5 * (TAU * k as f64 / n as f64).cos())
                .collect();
            let mut buf: Vec<Complex<f64>> = x
                .iter()
                .zip(&w)
                .map(|(s, w)| Complex::new(f64::from(s.re) * w, f64::from(s.im) * w))
                .collect();
            FftPlanner::<f64>::new()
                .plan_fft_forward(n)
                .process(&mut buf);
            let norm = n as f64 * w.iter().map(|w| w * w).sum::<f64>();
            Spectrum {
                p: buf.iter().map(|c| c.norm_sqr() / norm).collect(),
                rate,
            }
        }

        fn freq(&self, bin: usize) -> f64 {
            let n = self.p.len();
            let b = if bin < n / 2 {
                bin as f64
            } else {
                bin as f64 - n as f64
            };
            b * self.rate / n as f64
        }

        /// The strongest tone: its frequency and amplitude.
        fn peak(&self) -> (f64, f64) {
            let n = self.p.len();
            let k = (0..n)
                .max_by(|&a, &b| self.p[a].total_cmp(&self.p[b]))
                .unwrap();
            let amp2: f64 = (-3i64..=3)
                .map(|d| self.p[(k as i64 + d).rem_euclid(n as i64) as usize])
                .sum();
            (self.freq(k), amp2.sqrt())
        }

        /// Power with |f| below `hz`.
        fn power_within(&self, hz: f64) -> f64 {
            (0..self.p.len())
                .filter(|&k| self.freq(k).abs() < hz)
                .map(|k| self.p[k])
                .sum()
        }
    }

    fn db(x: f64) -> f64 {
        10.0 * x.max(1e-30).log10()
    }

    /// The tone at RF offset `delta` from the tuned centre, as the R820T
    /// presents it on its IF under libairspy's assumption: at 5 MHz − δ.
    fn rf_offset_tone(delta: f64, amp: f64, n: usize) -> Vec<u8> {
        raw_tone(5e6 - delta, amp, n)
    }

    #[test]
    fn libairspys_halfband_is_what_the_spec_says() {
        // The 31-tap kernel: the 16 listed taps at even positions, 0.5 at
        // the centre, zeros between.
        let mut h = [0f64; 31];
        for (j, &t) in HB.iter().enumerate() {
            h[2 * j] = f64::from(t);
        }
        h[15] = f64::from(HB_CENTRE);
        for (k, v) in h.iter().enumerate() {
            assert_eq!(*v == 0.0, k % 2 == 1 && k != 15, "tap {k}");
            assert!((v - h[30 - k]).abs() < 1e-12, "symmetry at {k}");
        }
        let n = 8192;
        let mut buf: Vec<Complex<f64>> = (0..n)
            .map(|k| Complex::new(h.get(k).copied().unwrap_or(0.0), 0.0))
            .collect();
        FftPlanner::<f64>::new()
            .plan_fft_forward(n)
            .process(&mut buf);
        let mag_db = |hz: f64| 20.0 * buf[(hz / FS * n as f64).round() as usize].norm().log10();
        // Flat to 3.9 MHz.
        let flat: Vec<f64> = (0..=(3.9e6 / FS * n as f64) as usize)
            .map(|k| 20.0 * buf[k].norm().log10())
            .collect();
        let (lo, hi) = flat
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        assert!(hi - lo < 0.02, "ripple {} dB", hi - lo);
        assert!(mag_db(0.0).abs() < 0.01, "DC gain {} dB", mag_db(0.0));
        // Better than 58 dB down from 6.1 MHz to the Nyquist frequency.
        let worst = (((6.1e6 / FS * n as f64) as usize)..=n / 2)
            .map(|k| 20.0 * buf[k].norm().log10())
            .fold(f64::MIN, f64::max);
        assert!(worst < -58.0, "stop band {worst} dB");
    }

    #[test]
    fn the_band_comes_out_the_way_round_libairspy_puts_it() {
        // RF at centre ± 1 MHz is on the IF at 5 MHz ∓ 1 MHz. With libairspy's
        // pattern it lands at ±1 MHz; with MIRROR_SPECTRUM it is conjugated.
        let sign = if MIRROR_SPECTRUM { -1.0 } else { 1.0 };
        for rate in OUTPUT_RATES_HZ {
            // ±1 MHz is beyond a 2.048 MS/s output's edge: ±0.7 MHz there.
            let edge = if rate < 4_000_000 { 0.7e6 } else { 1e6 };
            for delta in [edge, -edge, 0.5e6, -0.3e6] {
                let y = convert(rate, &rf_offset_tone(delta, 0.5, 400_000));
                let (f, a) = Spectrum::of(&y, f64::from(rate)).peak();
                assert!(
                    (f - sign * delta).abs() < 1.5 * f64::from(rate) / 32768.0,
                    "{rate} Hz, RF offset {delta}: landed at {f}"
                );
                assert!((a - 0.5).abs() < 0.02, "amplitude {a}");
            }
        }
    }

    #[test]
    fn a_full_scale_tone_gives_unit_amplitude() {
        // Same range as a full-scale cs8 complex tone (|IQ| = 1.0).
        let y = convert(4_096_000, &rf_offset_tone(0.2e6, 0.999, 400_000));
        let (_, a) = Spectrum::of(&y, 4.096e6).peak();
        assert!((a - 1.0).abs() < 0.02, "{a}");
        let peak = y[2000..].iter().map(|s| s.norm()).fold(0f32, f32::max);
        assert!((0.95..=1.05).contains(&peak), "{peak}");
    }

    #[test]
    fn the_ensemble_is_flat() {
        for rate in OUTPUT_RATES_HZ {
            let amp_at = |d: f64| {
                let y = convert(rate, &rf_offset_tone(d, 0.5, 400_000));
                Spectrum::of(&y, f64::from(rate)).peak().1
            };
            let reference = amp_at(0.05e6);
            for d in [-0.768e6, -0.5e6, -0.2e6, 0.2e6, 0.5e6, 0.768e6] {
                let dev = 20.0 * (amp_at(d) / reference).log10();
                assert!(dev.abs() < 0.1, "{rate} Hz at {d}: {dev} dB");
            }
        }
    }

    #[test]
    fn what_aliases_into_the_ensemble_is_below_minus_50_db() {
        // Tones all over the 20 MS/s Nyquist range, none in the ensemble:
        // whatever lands within ±0.77 MHz of the output is aliasing.
        for rate in OUTPUT_RATES_HZ {
            let mut worst = f64::MIN;
            let mut at = 0.0;
            // RF offsets from 0.9 to 5 MHz either side.
            for step in 0..=82 {
                let d = 0.9e6 + step as f64 * 0.05e6;
                for sign in [1.0, -1.0] {
                    let y = convert(rate, &rf_offset_tone(sign * d, 0.9, 500_000));
                    let p = Spectrum::of(&y, f64::from(rate)).power_within(0.77e6);
                    // Against the 0.9-scale tone itself: 0.9² of power.
                    let rel = db(p / 0.81);
                    if rel > worst {
                        worst = rel;
                        at = sign * d;
                    }
                }
            }
            eprintln!("{rate}: worst in-band alias {worst:.1} dBc at offset {at}");
            assert!(worst < -50.0, "{rate}: {worst} dBc at {at}");
        }
    }

    #[test]
    fn a_dc_offset_in_the_adc_does_not_reach_the_output() {
        for off in [0u16, 40, 300] {
            let raw: Vec<u8> = (0..400_000)
                .flat_map(|_| (2048u16 + off).to_le_bytes())
                .collect();
            let y = convert(4_096_000, &raw);
            let rms = (y[10_000..].iter().map(|s| s.norm_sqr()).sum::<f32>()
                / (y.len() - 10_000) as f32)
                .sqrt();
            assert!(rms < 1e-3, "offset {off}: rms {rms}");
        }
    }

    #[test]
    fn packed_input_gives_the_same_samples() {
        let raw = rf_offset_tone(0.4e6, 0.7, 64_000);
        let a = convert(4_096_000, &raw);
        let mut c = IqConverter::new(4_096_000, true).unwrap();
        let mut b = Vec::new();
        c.process(&pack(&raw), &mut b);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).norm() < 1e-6));
    }

    #[test]
    fn any_split_of_the_input_gives_the_same_output() {
        let raw = rf_offset_tone(0.3e6, 0.6, 100_000);
        for rate in OUTPUT_RATES_HZ {
            let whole = convert(rate, &raw);
            for pieces in [vec![1usize], vec![7, 100, 13], vec![4093, 16, 1, 262_144]] {
                let mut c = IqConverter::new(rate, false).unwrap();
                let mut out = Vec::new();
                let (mut at, mut k) = (0, 0);
                while at < raw.len() {
                    let n = pieces[k % pieces.len()].min(raw.len() - at);
                    c.process(&raw[at..at + n], &mut out);
                    at += n;
                    k += 1;
                }
                assert_eq!(out.len(), whole.len(), "{rate} {pieces:?}");
                // Only the DC estimate's update points move with the split.
                let worst = whole
                    .iter()
                    .zip(&out)
                    .map(|(a, b)| (a - b).norm())
                    .fold(0f32, f32::max);
                assert!(worst < 1e-3, "{rate} {pieces:?}: {worst}");
            }
        }
    }

    #[test]
    fn the_output_rate_is_what_was_asked() {
        for rate in OUTPUT_RATES_HZ {
            let y = convert(rate, &raw_tone(5e6, 0.5, 2_000_000));
            let want = 2_000_000.0 / FS * f64::from(rate);
            assert!(
                (y.len() as f64 - want).abs() < 4.0,
                "{rate}: {} samples, wanted {want}",
                y.len()
            );
        }
        assert!(IqConverter::new(3_000_000, false).is_err());
    }

    #[test]
    fn the_resampler_has_unit_gain_in_every_phase() {
        for (i, o) in [(10e6, 8.192e6), (5e6, 4.096e6), (2.5e6, 2.048e6)] {
            let r = PolyResampler::new(i, o).unwrap();
            for phi in 0..r.up {
                let s: f32 = r.rows[phi * r.t..(phi + 1) * r.t].iter().sum();
                assert!((s - 1.0).abs() < 0.01, "{i}→{o} phase {phi}: {s}");
            }
        }
    }

    #[test]
    fn every_half_band_variant_works() {
        // Zero to two half-bands in front of the resampler (where the output
        // allows it) all give the same ensemble.
        for (rate, max) in [(8_192_000u32, 0usize), (4_096_000, 1), (2_048_000, 2)] {
            for nh in 0..=max {
                let mut c = IqConverter::with_halfbands(rate, false, nh).unwrap();
                let mut y = Vec::new();
                c.process(&rf_offset_tone(0.6e6, 0.5, 400_000), &mut y);
                let (f, a) = Spectrum::of(&y, f64::from(rate)).peak();
                assert!(
                    (f - 0.6e6).abs() < 500.0 && (a - 0.5).abs() < 0.02,
                    "{rate}/{nh}"
                );
            }
        }
        assert!(IqConverter::with_halfbands(4_096_000, false, 2).is_err());
    }

    #[test]
    fn a_stream_passes_buffers_through_and_restarts_after_a_retune() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let (rtx, rrx) = mpsc::channel();
        let mut s = RawStream::new(rx, rtx, IqConverter::new(4_096_000, false).unwrap(), "fake");
        s.set_timeout(Duration::from_millis(100));
        let block = rf_offset_tone(0.5e6, 0.5, 131_072);
        let mut out = Vec::new();
        tx.send(block.clone()).unwrap();
        let n = s.read(&mut out).unwrap();
        assert_eq!(n, out.len());
        assert!((n as f64 - 131_072.0 * 4.096 / 20.0).abs() < 4.0);
        // The buffer came back for re-use.
        assert_eq!(rrx.try_recv().unwrap().len(), block.len());

        // A retune: five queued buffers go, then the next two are skipped.
        for _ in 0..5 {
            tx.send(vec![0u8; 16]).unwrap();
        }
        s.restart(2);
        tx.send(vec![1u8; 16]).unwrap();
        tx.send(vec![2u8; 16]).unwrap();
        tx.send(block.clone()).unwrap();
        let n = s.read(&mut out).unwrap();
        assert!(n > 26_000, "{n}");
        // Skipped and drained buffers were all handed back too.
        assert_eq!(rrx.try_iter().count(), 5 + 2 + 1);

        // Nothing arriving is an error, as is the reader going away.
        assert!(s.read(&mut out).is_err());
        drop(tx);
        assert!(
            s.read(&mut out)
                .unwrap_err()
                .to_string()
                .contains("reader stopped")
        );
    }

    /// The cost of the whole chain, release build:
    /// `cargo test -p decdab-io --release bench_converter -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing, not a check"]
    fn bench_converter() {
        let raw = rf_offset_tone(4.5e6, 0.5, 1 << 18);
        let packed = pack(&raw);
        for (rate, packed_in, nh) in [
            (4_096_000u32, false, 0usize),
            (4_096_000, false, 1),
            (4_096_000, true, 1),
            (2_048_000, false, 1),
            (2_048_000, false, 2),
            (8_192_000, false, 0),
        ] {
            let mut c = IqConverter::with_halfbands(rate, packed_in, nh).unwrap();
            let mut out = Vec::new();
            let input: &[u8] = if packed_in { &packed } else { &raw };
            for _ in 0..4 {
                out.clear();
                c.process(input, &mut out);
            }
            // The best of several rounds: a shared machine is noisy.
            let reps = 40;
            let mut secs = f64::MAX;
            let mut produced = 0usize;
            for _ in 0..9 {
                let t0 = std::time::Instant::now();
                produced = 0;
                for _ in 0..reps {
                    out.clear();
                    c.process(input, &mut out);
                    produced += out.len();
                }
                secs = secs.min(t0.elapsed().as_secs_f64());
            }
            let raw_samples = (reps * (1 << 18)) as f64;
            eprintln!(
                "{:>5.3} MS/s out, {} in, {} half-band(s): {:.2} ns/raw sample, {:.1} ns/output sample, {:.1} % of one core at 20 MS/s in",
                f64::from(rate) / 1e6,
                if packed_in { "packed  " } else { "unpacked" },
                nh,
                secs / raw_samples * 1e9,
                secs / produced as f64 * 1e9,
                secs / raw_samples * 20e6 * 100.0
            );
        }
    }
}
