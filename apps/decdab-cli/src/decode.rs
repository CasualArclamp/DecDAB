// made by CasualArclamp
//! `decdab decode`: one service of an ensemble to a WAV file, with the
//! decoding figures (CPU per stage, Viterbi, RS, AU and frame counts).

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use decdab_codecs::{AacDecoder, Mp2Decoder, Pcm};
use decdab_fic::{Ensemble, FicDecoder, Service};
use decdab_msc::{SubChannelDecoder, SuperframeDecoder};
use decdab_ofdm::params::{CIF_BITS, CIFS, FIC_BITS};
use decdab_ofdm::{Demod, Frontend};

use crate::Gains;

#[derive(clap::Args)]
pub struct DecodeArgs {
    /// An IQ file, or a block ("12B") / frequency in MHz for the HackRF.
    pub source: String,
    /// The service: part of its label ("Radio 4") or its SId in hex
    /// ("C224"). Without it, the services are listed.
    #[arg(short, long)]
    pub service: Option<String>,
    /// Seconds to decode.
    #[arg(long, default_value_t = 1e9)]
    pub seconds: f64,
    /// WAV file to write.
    #[arg(short, long)]
    pub out: Option<PathBuf>,
    #[command(flatten)]
    pub gains: Gains,
}

/// A 16-bit PCM WAV writer that fixes its sizes on `finish`.
struct Wav {
    f: std::io::BufWriter<std::fs::File>,
    rate: u32,
    channels: u16,
    bytes: u32,
}

impl Wav {
    fn create(path: &Path, rate: u32, channels: u16) -> Result<Wav> {
        let mut w = Wav {
            f: std::io::BufWriter::new(std::fs::File::create(path)?),
            rate,
            channels,
            bytes: 0,
        };
        w.header()?;
        Ok(w)
    }

    fn header(&mut self) -> Result<()> {
        let block = self.channels as u32 * 2;
        let f = &mut self.f;
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + self.bytes).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&self.channels.to_le_bytes())?;
        f.write_all(&self.rate.to_le_bytes())?;
        f.write_all(&(self.rate * block).to_le_bytes())?;
        f.write_all(&(block as u16).to_le_bytes())?;
        f.write_all(&16u16.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&self.bytes.to_le_bytes())?;
        Ok(())
    }

    fn write(&mut self, s: &[i16]) -> Result<()> {
        for v in s {
            self.f.write_all(&v.to_le_bytes())?;
        }
        self.bytes += 2 * s.len() as u32;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.f.seek(SeekFrom::Start(0))?;
        self.header()?;
        self.f.flush()?;
        Ok(())
    }
}

/// The service a pattern names: SId in hex, or part of the label.
pub fn find_service<'a>(e: &'a Ensemble, pat: &str) -> Option<&'a Service> {
    if let Ok(sid) = u32::from_str_radix(pat.trim(), 16)
        && let Some(s) = e.services.get(&sid)
    {
        return Some(s);
    }
    let p = pat.to_lowercase().replace(' ', "");
    e.services_by_name()
        .into_iter()
        .find(|s| s.name().to_lowercase().replace(' ', "").contains(&p))
}

enum Audio {
    Plus {
        sf: Box<SuperframeDecoder>,
        aac: Option<AacDecoder>,
    },
    Mp2(Box<Mp2Decoder>),
}

pub fn run(a: DecodeArgs) -> Result<()> {
    let mut src = crate::source::open(&a.source, a.gains)?;
    let rate = src.sample_rate();
    let mut fe = Frontend::new(rate).map_err(anyhow::Error::msg)?;
    let mut demod = Demod::new();
    let mut fic = FicDecoder::new();
    let mut sub: Option<(SubChannelDecoder, Audio, String)> = None;
    let mut wav: Option<Wav> = None;
    let out_path = a.out.clone();
    let (mut buf, mut dec) = (Vec::new(), Vec::new());
    let mut read = 0usize;
    let mut frames = 0u64;
    let (mut t_front, mut t_demod, mut t_fic, mut t_msc, mut t_audio) = (0f64, 0f64, 0f64, 0f64, 0f64);
    let mut pcm_secs = 0f64;
    let mut concealed = 0u64;
    let mut errors: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let mut give_up = false;
    while (read as f64) < a.seconds * rate && !give_up {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        read += n;
        let t = Instant::now();
        fe.process(&buf, &mut dec);
        t_front += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let mut work: Vec<Vec<i8>> = Vec::new();
        demod.push(&dec, |f| work.push(f.soft.to_vec()));
        t_demod += t.elapsed().as_secs_f64();
        for soft in work {
            frames += 1;
            let t = Instant::now();
            fic.push_frame(&soft[..FIC_BITS]);
            t_fic += t.elapsed().as_secs_f64();
            // Pick the service once the FIC has had a second to describe it.
            if sub.is_none() && frames >= 10 {
                let Some(pat) = &a.service else {
                    give_up = true;
                    break;
                };
                let e = &fic.ensemble;
                let Some(svc) = find_service(e, pat) else {
                    if frames > 40 {
                        bail!("no service matching {pat:?} in {}", e.name());
                    }
                    continue;
                };
                let comp = svc.primary().context("service has no components")?;
                let Some(sc) = e.subchannel_of(comp) else {
                    continue;
                };
                let d = SubChannelDecoder::new(*sc)
                    .with_context(|| format!("sub-channel {} has no valid protection", sc.id))?;
                let audio = if comp.is_dab_plus() {
                    Audio::Plus {
                        sf: Box::new(SuperframeDecoder::new(d.frame_bytes())),
                        aac: None,
                    }
                } else {
                    Audio::Mp2(Box::new(Mp2Decoder::new()))
                };
                eprintln!(
                    "{}: {} — sub-channel {}, {} kbit/s, {}, {}",
                    e.name(),
                    svc.name(),
                    sc.id,
                    sc.bitrate(),
                    sc.protection_name(),
                    if comp.is_dab_plus() { "DAB+" } else { "MP2" }
                );
                sub = Some((d, audio, svc.name()));
            }
            let Some((d, audio, _)) = &mut sub else {
                continue;
            };
            for c in 0..CIFS {
                let cif = &soft[FIC_BITS + c * CIF_BITS..FIC_BITS + (c + 1) * CIF_BITS];
                let t = Instant::now();
                let lf = d.push_cif(cif).map(|f| f.to_vec());
                t_msc += t.elapsed().as_secs_f64();
                let Some(lf) = lf else { continue };
                let t = Instant::now();
                let mut pcms: Vec<Pcm> = Vec::new();
                match audio {
                    Audio::Plus { sf, aac } => {
                        sf.push(&lf, |p, au| {
                            if aac.is_none() {
                                match AacDecoder::new(&p.audio_specific_config()) {
                                    Ok(x) => *aac = Some(x),
                                    Err(e) => errors.push(e.to_string()),
                                }
                            }
                            if let Some(dec) = aac {
                                match dec.decode(au.ok.then_some(au.data)) {
                                    Ok(pcm) => pcms.push(pcm),
                                    Err(e) => errors.push(e.to_string()),
                                }
                            }
                        });
                    }
                    Audio::Mp2(m) => {
                        let mut found: Vec<Vec<u8>> = Vec::new();
                        m.push(&lf, |fr| found.push(fr.to_vec()));
                        for fr in found {
                            match m.decode(&fr) {
                                Ok(p) => pcms.push(p),
                                Err(e) => errors.push(e.to_string()),
                            }
                        }
                    }
                }
                t_audio += t.elapsed().as_secs_f64();
                for p in pcms {
                    concealed += p.concealed as u64;
                    pcm_secs += p.samples.len() as f64 / p.channels as f64 / p.rate as f64;
                    if let Some(path) = &out_path {
                        if wav.is_none() {
                            wav = Some(Wav::create(path, p.rate, p.channels)?);
                        }
                        if let Some(w) = &mut wav {
                            w.write(&p.samples)?;
                        }
                    }
                }
            }
        }
    }
    drop(src);
    if let Some(w) = wav {
        w.finish()?;
    }
    if a.service.is_none() {
        println!("services in {} (pick one with --service):", fic.ensemble.name());
        for s in fic.ensemble.services_by_name() {
            println!("  {:04X}  {}", s.sid, s.name());
        }
        return Ok(());
    }
    let secs = read as f64 / rate;
    let wall = t0.elapsed().as_secs_f64();
    let pc = |t: f64| t / secs * 100.0;
    println!(
        "{:.1} s of signal in {:.2} s: {:.1} s of audio ({} frames concealed); FIBs {}/{}",
        secs, wall, pcm_secs, concealed, fic.stats.fibs_ok, fic.stats.fibs
    );
    if let Some((d, audio, name)) = &sub {
        print!("{name}: {} logical frames", d.frames);
        match audio {
            Audio::Plus { sf, .. } => {
                let s = sf.stats;
                let codec = sf.params.map_or("?", |p| p.codec_name());
                println!(
                    ", {} superframes ({codec}), RS corrected {} bytes, {} uncorrectable; AUs {} good, {} bad",
                    s.superframes, s.rs_corrected, s.rs_failed, s.au_ok, s.au_bad
                );
            }
            Audio::Mp2(m) => println!(", {} MP2 frames, {} bad", m.frames, m.bad),
        }
    }
    println!(
        "CPU (% of a core): front end {:.2}, OFDM {:.2}, FIC {:.2}, sub-channel {:.2}, audio {:.2}; whole run {:.1}",
        pc(t_front),
        pc(t_demod),
        pc(t_fic),
        pc(t_msc),
        pc(t_audio),
        pc(wall)
    );
    if !errors.is_empty() {
        errors.dedup();
        println!("{} decoder errors, first: {}", errors.len(), errors[0]);
    }
    Ok(())
}
