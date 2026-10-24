// made by CasualArclamp
//! MPEG-1 and MPEG-2 (low sampling frequency) Audio Layer II, the codec of
//! DAB's original audio services (EN 300 401 §7; ISO/IEC 11172-3 §2.4,
//! ISO/IEC 13818-3).
//!
//! A frame holds 1152 samples a channel: a header, the bit allocation of
//! each sub-band, scale factor selection, scale factors (three per
//! sub-band, one for each 384-sample part), then 12 granules of three
//! samples per sub-band. The polyphase synthesis is the standard's own
//! (Figure 3-A.2): matrixing into V, windowing with D, summing.
//!
//! In DAB a 48 kHz frame fills one logical frame (24 ms) exactly and a
//! 24 kHz frame two; [`Mp2Decoder::push`] finds frames in the byte stream
//! either way.

use std::sync::OnceLock;

use crate::mp2_window::SYNTHESIS_D;
use crate::{CodecError, Pcm};

/// Quantisation classes (Table 3-B.4): levels, grouped, bits per sample
/// or per group of three.
const CLASSES: [(u32, bool, u32); 17] = [
    (3, true, 5),
    (5, true, 7),
    (7, false, 3),
    (9, true, 10),
    (15, false, 4),
    (31, false, 5),
    (63, false, 6),
    (127, false, 7),
    (255, false, 8),
    (511, false, 9),
    (1023, false, 10),
    (2047, false, 11),
    (4095, false, 12),
    (8191, false, 13),
    (16383, false, 14),
    (32767, false, 15),
    (65535, false, 16),
];

/// The allocation rows of Tables 3-B.2a–d and 13818-3 B.1: nbal, and the
/// class of each allocation value (index 0 is "none").
const ROWS: [(u32, [u8; 16]); 8] = [
    (2, [0, 0, 1, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    (2, [0, 0, 1, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    (3, [0, 0, 1, 3, 4, 5, 6, 7, 0, 0, 0, 0, 0, 0, 0, 0]),
    (3, [0, 0, 1, 2, 3, 4, 5, 16, 0, 0, 0, 0, 0, 0, 0, 0]),
    (4, [0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]),
    (4, [0, 0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
    (4, [0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16]),
    (4, [0, 0, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]),
];

/// For each table, sblimit and the row of each sub-band.
const TABLES: [(usize, [u8; 30]); 5] = [
    // 3-B.2a
    (
        27,
        [
            7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0,
        ],
    ),
    // 3-B.2b
    (
        30,
        [
            7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 0, 0, 0, 0, 0, 0, 0,
        ],
    ),
    // 3-B.2c
    (
        8,
        [
            5, 5, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    ),
    // 3-B.2d
    (
        12,
        [
            5, 5, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    ),
    // 13818-3 B.1 (low sampling frequencies)
    (
        30,
        [
            4, 4, 4, 4, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        ],
    ),
];

const BITRATES_MPEG1: [u32; 15] = [
    0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
];
const BITRATES_MPEG2: [u32; 15] = [
    0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160,
];

/// A Layer II frame header (§2.4.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// MPEG-1 (else MPEG-2 low sampling frequency).
    pub mpeg1: bool,
    pub crc: bool,
    pub bitrate: u32,
    pub rate: u32,
    pub padding: bool,
    /// 0 stereo, 1 joint stereo, 2 dual channel, 3 mono.
    pub mode: u8,
    pub mode_ext: u8,
}

impl Header {
    pub fn parse(b: &[u8]) -> Option<Header> {
        if b.len() < 4 || b[0] != 0xFF || b[1] & 0xF0 != 0xF0 {
            return None;
        }
        let mpeg1 = b[1] & 0x08 != 0;
        if (b[1] >> 1) & 3 != 2 {
            return None; // not Layer II
        }
        let bi = (b[2] >> 4) as usize;
        let si = ((b[2] >> 2) & 3) as usize;
        if bi == 0 || bi == 15 || si == 3 {
            return None;
        }
        let bitrate = if mpeg1 {
            BITRATES_MPEG1[bi]
        } else {
            BITRATES_MPEG2[bi]
        };
        let rate = if mpeg1 {
            [44_100, 48_000, 32_000][si]
        } else {
            [22_050, 24_000, 16_000][si]
        };
        Some(Header {
            mpeg1,
            crc: b[1] & 1 == 0,
            bitrate,
            rate,
            padding: b[2] & 2 != 0,
            mode: b[3] >> 6,
            mode_ext: (b[3] >> 4) & 3,
        })
    }

    /// Bytes in the frame.
    pub fn frame_len(&self) -> usize {
        (144 * self.bitrate * 1000 / self.rate) as usize + self.padding as usize
    }

    pub fn channels(&self) -> usize {
        if self.mode == 3 { 1 } else { 2 }
    }

    /// Which allocation table (ISO/IEC 11172-3 §2.4.3.3.1).
    fn table(&self) -> usize {
        if !self.mpeg1 {
            return 4;
        }
        let per_ch = self.bitrate / self.channels() as u32;
        if (self.rate == 48_000 && per_ch >= 56) || (56..=80).contains(&per_ch) {
            0
        } else if self.rate != 48_000 && per_ch >= 96 {
            1
        } else if self.rate != 32_000 && per_ch <= 48 {
            2
        } else {
            3
        }
    }
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            let byte = self.d.get(self.pos / 8).copied().unwrap_or(0);
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u32;
            self.pos += 1;
        }
        v
    }
}

/// The synthesis matrix N[i][k] = cos((16 + i)(2k + 1)π/64).
fn matrix() -> &'static [[f32; 32]; 64] {
    static N: OnceLock<Box<[[f32; 32]; 64]>> = OnceLock::new();
    N.get_or_init(|| {
        let mut n = Box::new([[0f32; 32]; 64]);
        for (i, row) in n.iter_mut().enumerate() {
            for (k, v) in row.iter_mut().enumerate() {
                *v = (((16 + i) * (2 * k + 1)) as f64 * std::f64::consts::PI / 64.0).cos() as f32;
            }
        }
        n
    })
}

/// One channel's synthesis filterbank state.
struct Synth {
    v: [f32; 1024],
    off: usize,
}

impl Synth {
    fn new() -> Self {
        Synth {
            v: [0.0; 1024],
            off: 0,
        }
    }

    /// 32 sub-band samples in, 32 PCM samples out.
    fn run(&mut self, s: &[f32; 32], out: &mut [f32; 32]) {
        let n = matrix();
        self.off = (self.off + 1024 - 64) % 1024;
        for (i, row) in n.iter().enumerate() {
            let mut acc = 0f32;
            for k in 0..32 {
                acc += row[k] * s[k];
            }
            self.v[self.off + i] = acc;
        }
        // U from V (Figure 3-A.2), windowed and summed in one pass.
        for (j, o) in out.iter_mut().enumerate() {
            let mut acc = 0f32;
            for i in 0..8 {
                let a = (self.off + 128 * i + j) % 1024;
                let b = (self.off + 128 * i + 96 + j) % 1024;
                acc += self.v[a] * SYNTHESIS_D[64 * i + j];
                acc += self.v[b] * SYNTHESIS_D[64 * i + 32 + j];
            }
            *o = acc;
        }
    }
}

pub struct Mp2Decoder {
    synth: [Synth; 2],
    buf: Vec<u8>,
    pub frames: u64,
    pub bad: u64,
}

impl Default for Mp2Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp2Decoder {
    pub fn new() -> Self {
        Mp2Decoder {
            synth: [Synth::new(), Synth::new()],
            buf: Vec::new(),
            frames: 0,
            bad: 0,
        }
    }

    /// Take stream bytes; `on_frame` gets each whole frame found (its bytes,
    /// header included), for PAD extraction and decoding.
    pub fn push(&mut self, bytes: &[u8], mut on_frame: impl FnMut(&[u8])) {
        self.buf.extend_from_slice(bytes);
        let mut i = 0;
        while i + 4 <= self.buf.len() {
            let Some(h) = Header::parse(&self.buf[i..]) else {
                i += 1;
                continue;
            };
            let len = h.frame_len();
            if i + len > self.buf.len() {
                break;
            }
            on_frame(&self.buf[i..i + len]);
            i += len;
        }
        self.buf.drain(..i);
    }

    /// Decode one frame to 1152 samples a channel, always stereo out.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Pcm, CodecError> {
        let h = Header::parse(frame).ok_or(CodecError::Bad("not a Layer II frame"))?;
        if frame.len() < h.frame_len() {
            return Err(CodecError::Bad("short frame"));
        }
        let nch = h.channels();
        let (sblimit, rows) = TABLES[h.table()];
        let bound = match h.mode {
            1 => (4 * (h.mode_ext as usize + 1)).min(sblimit),
            3 => 0,
            _ => sblimit,
        };
        let mut b = Bits {
            d: frame,
            pos: 32 + if h.crc { 16 } else { 0 },
        };
        // Bit allocation.
        let mut alloc = [[0u8; 32]; 2];
        for sb in 0..sblimit {
            let (nbal, _) = ROWS[rows[sb] as usize];
            if sb < bound || nch == 1 {
                for a in alloc.iter_mut().take(nch) {
                    a[sb] = b.get(nbal) as u8;
                }
            } else {
                let v = b.get(nbal) as u8;
                alloc[0][sb] = v;
                alloc[1][sb] = v;
            }
        }
        // Scale factor selection, then the scale factors.
        let mut scfsi = [[0u8; 32]; 2];
        for sb in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][sb] != 0 {
                    scfsi[ch][sb] = b.get(2) as u8;
                }
            }
        }
        let mut scf = [[[0f32; 3]; 32]; 2];
        for sb in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][sb] == 0 {
                    continue;
                }
                let mut idx = [0u32; 3];
                match scfsi[ch][sb] {
                    0 => {
                        for v in &mut idx {
                            *v = b.get(6);
                        }
                    }
                    1 => {
                        let a = b.get(6);
                        let c = b.get(6);
                        idx = [a, a, c];
                    }
                    2 => {
                        let a = b.get(6);
                        idx = [a, a, a];
                    }
                    _ => {
                        let a = b.get(6);
                        let c = b.get(6);
                        idx = [a, c, c];
                    }
                }
                for (p, &i) in idx.iter().enumerate() {
                    // Table 3-B.1: 2^(1 − i/3); 63 is not allowed.
                    scf[ch][sb][p] = if i < 63 {
                        2f32.powf(1.0 - i as f32 / 3.0)
                    } else {
                        0.0
                    };
                }
            }
        }
        // Samples: 12 granules of three per sub-band, then synthesis.
        let mut out = Vec::with_capacity(1152 * 2);
        let mut pcm = [[[0f32; 32]; 3]; 2];
        for gr in 0..12 {
            let part = gr / 4;
            let mut sub = [[[0f32; 32]; 3]; 2];
            for sb in 0..sblimit {
                let row = ROWS[rows[sb] as usize].1;
                let chans = if sb < bound || nch == 1 { nch } else { 1 };
                for ch in 0..chans {
                    let a = alloc[ch][sb] as usize;
                    if a == 0 {
                        continue;
                    }
                    let (levels, grouped, bits) = CLASSES[row[a] as usize];
                    let mut codes = [0u32; 3];
                    if grouped {
                        let mut c = b.get(bits);
                        for v in &mut codes {
                            *v = c % levels;
                            c /= levels;
                        }
                    } else {
                        for v in &mut codes {
                            *v = b.get(bits);
                        }
                    }
                    // §2.4.3.3.5: the requantised value works out to
                    // (2c − (L − 1)) / L for every class.
                    let fl = levels as f32;
                    for (s, &c) in codes.iter().enumerate() {
                        let v = (2.0 * c as f32 - (fl - 1.0)) / fl;
                        sub[ch][s][sb] = v * scf[ch][sb][part];
                        if chans == 1 && nch == 2 {
                            sub[1][s][sb] = v * scf[1][sb][part];
                        }
                    }
                }
            }
            for s in 0..3 {
                for ch in 0..nch {
                    self.synth[ch].run(&sub[ch][s], &mut pcm[ch][s]);
                }
                for (j, &l) in pcm[0][s].iter().enumerate() {
                    let r = if nch == 2 { pcm[1][s][j] } else { l };
                    out.push((l * 32767.0).clamp(-32768.0, 32767.0) as i16);
                    out.push((r * 32767.0).clamp(-32768.0, 32767.0) as i16);
                }
            }
        }
        self.frames += 1;
        Ok(Pcm {
            rate: h.rate,
            channels: 2,
            samples: out,
            concealed: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dab_headers() {
        // MPEG-1 Layer II, no CRC... 128 kbit/s, 48 kHz, joint stereo.
        let h = Header::parse(&[0xFF, 0xFD, 0x84, 0x40]).unwrap();
        assert!(h.mpeg1 && !h.crc);
        assert_eq!((h.bitrate, h.rate, h.mode), (128, 48_000, 1));
        assert_eq!(h.frame_len(), 384); // one 24 ms logical frame
        assert_eq!(h.table(), 0);
        // MPEG-2 LSF, 64 kbit/s, 24 kHz, mono: two logical frames.
        let h = Header::parse(&[0xFF, 0xF5, 0x84, 0xC0]).unwrap();
        assert!(!h.mpeg1);
        assert_eq!((h.bitrate, h.rate, h.channels()), (64, 24_000, 1));
        assert_eq!(h.frame_len(), 384);
        assert_eq!(h.table(), 4);
    }

    #[test]
    fn silence_decodes_to_silence() {
        // A frame with no bits allocated anywhere.
        let mut f = vec![0u8; 384];
        f[..4].copy_from_slice(&[0xFF, 0xFD, 0x84, 0x40]);
        let pcm = Mp2Decoder::new().decode(&f).unwrap();
        assert_eq!(pcm.samples.len(), 2304);
        assert!(pcm.samples.iter().all(|&s| s == 0));
    }
}
