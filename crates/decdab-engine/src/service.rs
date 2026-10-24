// made by CasualArclamp
//! One service, from its sub-channel's CIFs to sound and PAD: the
//! sub-channel decoder, then DAB+ superframes and FDK-AAC, or MP2 frames
//! and the Layer II decoder; the PAD of either goes to the PAD decoder
//! (dynamic label, slideshow).

use decdab_codecs::{AacDecoder, Mp2Decoder, Pcm};
use decdab_data::pad::{PadDecoder, PadEvent};
use decdab_fic::{Component, SubChannel};
use decdab_msc::{AudioParams, SubChannelDecoder, SuperframeDecoder, au_pad};

/// Figures for the service's display.
#[derive(Debug, Clone, Default)]
pub struct ServiceStats {
    /// Logical frames out of the sub-channel decoder.
    pub frames: u64,
    /// DAB+: superframes, RS bytes corrected and blocks lost; AUs good and
    /// bad. MP2: frames decoded (as AUs good).
    pub superframes: u64,
    pub rs_corrected: u64,
    pub rs_failed: u64,
    pub au_ok: u64,
    pub au_bad: u64,
    /// Audio frames the codec concealed.
    pub concealed: u64,
    /// "HE-AAC v2 · 48 kHz", "MPEG-1 Layer II · 48 kHz".
    pub codec: String,
    pub synced: bool,
    pub errors: u64,
    pub last_error: Option<String>,
}

enum Audio {
    Plus {
        sf: Box<SuperframeDecoder>,
        aac: Option<(AudioParams, AacDecoder)>,
    },
    Mp2(Box<Mp2Decoder>),
}

pub struct ServiceDecoder {
    sub: SubChannelDecoder,
    audio: Audio,
    pad: PadDecoder,
    pub stats: ServiceStats,
}

impl ServiceDecoder {
    /// For a component carried in `sub` (an audio component; `None` for one
    /// that is not audio or whose protection is impossible).
    pub fn new(comp: &Component, sub: SubChannel) -> Option<Self> {
        let d = SubChannelDecoder::new(sub)?;
        let audio = if comp.is_dab_plus() {
            Audio::Plus {
                sf: Box::new(SuperframeDecoder::new(d.frame_bytes())),
                aac: None,
            }
        } else if matches!(comp.transport, decdab_fic::Transport::Audio { .. }) {
            Audio::Mp2(Box::new(Mp2Decoder::new()))
        } else {
            return None;
        };
        Some(ServiceDecoder {
            sub: d,
            audio,
            pad: PadDecoder::new(),
            stats: ServiceStats::default(),
        })
    }

    pub fn subchannel(&self) -> &SubChannel {
        self.sub.subchannel()
    }

    /// Take a CIF's soft bits; decoded audio goes to `pcm`, PAD events to
    /// `pad`.
    pub fn push_cif(&mut self, cif: &[i8], pcm: &mut Vec<Pcm>, pad: &mut Vec<PadEvent>) {
        let Some(lf) = self.sub.push_cif(cif) else {
            return;
        };
        self.stats.frames += 1;
        let st = &mut self.stats;
        let pad_dec = &mut self.pad;
        match &mut self.audio {
            Audio::Plus { sf, aac } => {
                sf.push(lf, |p, au| {
                    if aac.as_ref().is_none_or(|(q, _)| q != p) {
                        match AacDecoder::new(&p.audio_specific_config()) {
                            Ok(d) => {
                                *aac = Some((*p, d));
                                st.codec = format!(
                                    "{} · {} kHz{}",
                                    p.codec_name(),
                                    p.output_rate() / 1000,
                                    if p.stereo || p.ps { " stereo" } else { " mono" }
                                );
                            }
                            Err(e) => {
                                st.errors += 1;
                                st.last_error = Some(e.to_string());
                            }
                        }
                    }
                    if au.ok
                        && let Some(field) = au_pad(au.data)
                    {
                        pad_dec.push(field, pad);
                    }
                    if let Some((_, d)) = aac {
                        match d.decode(au.ok.then_some(au.data)) {
                            Ok(p) => {
                                st.concealed += p.concealed as u64;
                                pcm.push(p);
                            }
                            Err(e) => {
                                st.errors += 1;
                                st.last_error = Some(e.to_string());
                            }
                        }
                    }
                });
                let s = sf.stats;
                st.superframes = s.superframes;
                st.rs_corrected = s.rs_corrected;
                st.rs_failed = s.rs_failed;
                st.au_ok = s.au_ok;
                st.au_bad = s.au_bad;
                st.synced = s.synced;
            }
            Audio::Mp2(m) => {
                let mut frames: Vec<Vec<u8>> = Vec::new();
                m.push(lf, |f| frames.push(f.to_vec()));
                for f in frames {
                    if let Some(field) = decdab_codecs::mp2::pad_field(&f) {
                        pad_dec.push(&field, pad);
                    }
                    match m.decode(&f) {
                        Ok(p) => {
                            if st.codec.is_empty()
                                && let Some(h) = decdab_codecs::mp2::Header::parse(&f)
                            {
                                st.codec = format!(
                                    "MPEG-{} Layer II · {} kHz{}",
                                    if h.mpeg1 { 1 } else { 2 },
                                    h.rate / 1000,
                                    if h.channels() == 2 { " stereo" } else { " mono" }
                                );
                            }
                            st.au_ok += 1;
                            pcm.push(p);
                        }
                        Err(e) => {
                            st.au_bad += 1;
                            st.errors += 1;
                            st.last_error = Some(e.to_string());
                        }
                    }
                }
                st.synced = st.au_ok > 0;
            }
        }
    }
}
