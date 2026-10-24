// made by CasualArclamp
//! The receive thread: reads an [`IqSource`], builds waterfall rows and level
//! readings, runs the DAB receiver (OFDM, FIC, the chosen service) and the
//! sound, and writes a recording when asked. After DecDVB-T's engine.
//!
//! One thread, one pass over each block. The GUI never touches the source:
//! it sends [`Command`]s and takes rows, [`Stats`] and the receiver's
//! [`DabUpdate`] from [`Rx`].

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use decdab_audio::{AudioHandle, AudioPlayer, OutputKind};
use decdab_codecs::Pcm;
use decdab_core::{Iq, SampleFormat};
use decdab_data::pad::{DynamicLabel, PadEvent};
use decdab_fic::{Ensemble, FicDecoder};
use decdab_io::{IqFileWriter, IqSource};
use decdab_ofdm::params::{CIF_BITS, CIFS, FIC_BITS};
use decdab_ofdm::{Demod, Frontend, Scope};

use crate::frontend::{FrontEnd, FrontEndReading, FrontEndSettings};
use crate::meter::{Stage, StageLoad, StageMeter};
use crate::service::{ServiceDecoder, ServiceStats};
use crate::spectrum::RowBuilder;

/// How the spectrum is built, and where the sound goes.
#[derive(Debug, Clone, Copy)]
pub struct RxOptions {
    pub fft: usize,
    pub rows_per_s: f64,
    /// FFT segments averaged into each row: the waterfall's whole cost.
    pub segments_per_row: u32,
    /// Play the chosen service on the sound card (off: decode and count).
    pub sound: bool,
}

impl Default for RxOptions {
    fn default() -> Self {
        RxOptions {
            fft: 4096,
            rows_per_s: 20.0,
            segments_per_row: 16,
            sound: true,
        }
    }
}

/// One line of the waterfall.
#[derive(Debug, Clone)]
pub struct Row {
    /// dBFS per bin, the most negative frequency first.
    pub db: Vec<f32>,
    pub center_hz: f64,
    pub sample_rate: f64,
    /// Counts retunes: a row from before one does not line up with the
    /// rows after it.
    pub tuning: u32,
}

/// What the GUI is told about the front end, refreshed every second.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub describe: String,
    pub sample_rate: f64,
    pub center_hz: f64,
    pub samples: u64,
    /// RMS level of the input, dBFS (a full-scale complex sample is 0).
    pub rms_dbfs: f32,
    /// Highest |I| or |Q| in the second, as a fraction of full scale.
    pub peak: f32,
    /// Samples at the ADC's limit, percent.
    pub clip_percent: f32,
    /// Buffers the source had to drop because we fell behind.
    pub dropped: u64,
    pub front_end: FrontEndReading,
    pub load: StageLoad,
    pub recording: Option<Recording>,
    pub error: Option<String>,
    /// The source is exhausted (a file played to its end).
    pub ended: bool,
}

#[derive(Debug, Clone)]
pub struct Recording {
    pub path: PathBuf,
    pub bytes: u64,
}

pub enum Command {
    Retune(f64),
    /// Decode the ensemble at this RF frequency, Hz, or stop with `None`.
    Decode(Option<f64>),
    /// Play this service (its SId), or none.
    Select(Option<u32>),
    /// Make the scopes' view of each frame while on.
    Scopes(bool),
    /// Rows three times as often, each a third the work (60 fps display).
    Smooth(bool),
    FrontEnd(FrontEndSettings),
    /// Start recording to this file (cs8), or stop with `None`.
    Record(Option<PathBuf>),
}

/// A slideshow picture (or another MOT object) from the playing service.
#[derive(Debug, Clone)]
pub struct Slide {
    pub name: String,
    pub mime: String,
    pub data: Arc<Vec<u8>>,
    /// Counts the slides, so a GUI can tell a new one.
    pub serial: u64,
}

/// The receiver's state for the GUI.
#[derive(Debug, Clone, Default)]
pub struct DabStatus {
    /// The demodulator holds frame sync.
    pub locked: bool,
    pub frames: u64,
    pub mer_db: f32,
    pub cfo_hz: f64,
    /// The impulse response's peak over its mean, dB.
    pub peak_db: f32,
    /// FIBs with a good CRC over the last second, of all, and in total.
    pub fib_ok_ratio: f32,
    pub fibs_ok: u64,
    pub fibs: u64,
    /// The service playing (its SId), with its figures, label and picture.
    pub playing: Option<u32>,
    /// Its sub-channel (protection, size, bit rate).
    pub subchannel: Option<decdab_fic::SubChannel>,
    pub service: Option<ServiceStats>,
    pub dls: Option<DynamicLabel>,
    pub slide: Option<Slide>,
    /// The sound: its handle (state, levels, volume) while a service plays.
    pub audio: Option<AudioHandle>,
}

/// What [`Rx::take_dab`] hands over.
#[derive(Default)]
pub struct DabUpdate {
    pub status: DabStatus,
    /// The ensemble, when it has changed since the last call.
    pub ensemble: Option<Arc<Ensemble>>,
    /// The scopes' view of the newest frame, if one has come.
    pub scope: Option<Arc<Scope>>,
}

#[derive(Default)]
struct DabOut {
    status: DabStatus,
    ensemble: Option<Arc<Ensemble>>,
    scope: Option<Arc<Scope>>,
}

struct Shared {
    rows: Mutex<VecDeque<Row>>,
    stats: Mutex<Stats>,
    dab: Mutex<DabOut>,
    stop: AtomicBool,
}

/// Signal thrown away after a retune, seconds.
const SETTLE_S: f64 = 0.05;
/// Rows kept for a GUI that is slow to collect them.
const MAX_QUEUED_ROWS: usize = 100;

/// A running receive thread.
pub struct Rx {
    cmd: Sender<Command>,
    shared: Arc<Shared>,
    join: Option<JoinHandle<()>>,
}

/// Reads the source's own drop counter (a HackRF's USB overflows).
pub type DropProbe = Box<dyn Fn() -> u64 + Send>;

impl Rx {
    pub fn start(source: Box<dyn IqSource>, opts: RxOptions, drops: Option<DropProbe>) -> Rx {
        let shared = Arc::new(Shared {
            rows: Mutex::new(VecDeque::new()),
            stats: Mutex::new(Stats {
                describe: source.describe(),
                sample_rate: source.sample_rate(),
                center_hz: source.center_freq(),
                ..Default::default()
            }),
            dab: Mutex::new(DabOut::default()),
            stop: AtomicBool::new(false),
        });
        let (cmd, cmd_rx) = mpsc::channel();
        let sh = Arc::clone(&shared);
        let join = std::thread::Builder::new()
            .name("decdab-rx".into())
            .spawn(move || run(source, opts, drops, cmd_rx, sh))
            .expect("cannot start the receive thread");
        Rx {
            cmd,
            shared,
            join: Some(join),
        }
    }

    pub fn send(&self, c: Command) {
        // The thread may have ended (end of file, error); nothing to tell.
        let _ = self.cmd.send(c);
    }

    /// Rows made since the last call, oldest first.
    pub fn take_rows(&self) -> Vec<Row> {
        self.shared.rows.lock().unwrap().drain(..).collect()
    }

    pub fn stats(&self) -> Stats {
        self.shared.stats.lock().unwrap().clone()
    }

    /// The receiver's state, and what is new since the last call.
    pub fn take_dab(&self) -> DabUpdate {
        let mut o = self.shared.dab.lock().unwrap();
        DabUpdate {
            status: o.status.clone(),
            ensemble: o.ensemble.take(),
            scope: o.scope.take(),
        }
    }

    pub fn running(&self) -> bool {
        self.join.as_ref().is_some_and(|j| !j.is_finished())
    }
}

impl Drop for Rx {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

/// Level of a block, from every 8th sample.
#[derive(Default)]
struct Level {
    power: f64,
    n: u64,
    peak: f32,
    clipped: u64,
}

impl Level {
    const STRIDE: usize = 8;
    const CLIP: f32 = 0.99;

    fn add(&mut self, block: &[Iq]) {
        for s in block.iter().step_by(Self::STRIDE) {
            self.power += s.norm_sqr() as f64;
            let m = s.re.abs().max(s.im.abs());
            self.peak = self.peak.max(m);
            self.clipped += (m >= Self::CLIP) as u64;
            self.n += 1;
        }
    }
}

/// The receiver proper, while decoding.
struct Receiver_ {
    fe: Frontend,
    demod: Demod,
    fic: FicDecoder,
    fic_gen: u64,
    service: Option<(u32, ServiceDecoder)>,
    player: Option<AudioPlayer>,
    dec: Vec<Iq>,
    /// The FIB counts a second ago, for the ratio.
    fib_mark: (u64, u64, Instant),
    dls: Option<DynamicLabel>,
    slide: Option<Slide>,
    slides: u64,
}

impl Receiver_ {
    fn new(vfo_hz: f64, centre: f64, rate: f64) -> Result<Receiver_, String> {
        let offset = if centre > 0.0 { vfo_hz - centre } else { 0.0 };
        Ok(Receiver_ {
            fe: Frontend::with_offset(rate, offset)?,
            demod: Demod::new(),
            fic: FicDecoder::new(),
            fic_gen: u64::MAX,
            service: None,
            player: None,
            dec: Vec::new(),
            fib_mark: (0, 0, Instant::now()),
            dls: None,
            slide: None,
            slides: 0,
        })
    }
}

fn run(
    mut source: Box<dyn IqSource>,
    opts: RxOptions,
    drops: Option<DropProbe>,
    cmds: Receiver<Command>,
    sh: Arc<Shared>,
) {
    let rate = source.sample_rate();
    let live = source.is_live();
    let (mut rows_per_s, mut segments) = (opts.rows_per_s, opts.segments_per_row);
    let mut builder = RowBuilder::new(opts.fft, rate, rows_per_s, segments);
    let mut meter = StageMeter::default();
    let mut block: Vec<Iq> = Vec::new();
    let mut level = Level::default();
    let mut total = 0u64;
    let mut tuning = 0u32;
    let mut writer: Option<(IqFileWriter, PathBuf, u64)> = None;
    let mut rx: Option<Receiver_> = None;
    let mut want_service: Option<u32> = None;
    let mut scopes_on = false;
    let mut settle = 0u64;
    let mut front = FrontEnd::new(FrontEndSettings::default());
    let mut publish_at = Instant::now();
    let mut publish = Duration::from_millis(100);
    let mut row = Vec::new();
    let started = Instant::now();
    let mut pcm: Vec<Pcm> = Vec::new();
    let mut pad: Vec<PadEvent> = Vec::new();
    let mut frames: Vec<Vec<i8>> = Vec::new();
    let mut frame_stats = None;
    let mut new_scope: Option<Arc<Scope>> = None;
    let set_error = |e: String| sh.stats.lock().unwrap().error = Some(e);
    let new_rx = |vfo: f64, centre: f64, scopes: bool| match Receiver_::new(vfo, centre, rate) {
        Ok(mut r) => {
            r.demod.set_scopes(scopes);
            Ok(r)
        }
        Err(e) => Err(e),
    };

    while !sh.stop.load(Ordering::Relaxed) {
        loop {
            match cmds.try_recv() {
                Ok(Command::Decode(vfo)) => {
                    rx = None;
                    if let Some(v) = vfo {
                        match new_rx(v, source.center_freq(), scopes_on) {
                            Ok(r) => rx = Some(r),
                            Err(e) => set_error(e),
                        }
                    }
                    *sh.dab.lock().unwrap() = DabOut::default();
                }
                Ok(Command::Select(sid)) => {
                    want_service = sid;
                    if let Some(r) = &mut rx {
                        r.service = None;
                        r.player = None;
                        r.dls = None;
                        r.slide = None;
                    }
                }
                Ok(Command::Scopes(on)) => {
                    scopes_on = on;
                    if let Some(r) = &mut rx {
                        r.demod.set_scopes(on);
                    }
                }
                Ok(Command::FrontEnd(f)) => front.settings = f,
                Ok(Command::Smooth(on)) => {
                    (rows_per_s, segments, publish) = if on {
                        (
                            opts.rows_per_s * 3.0,
                            (opts.segments_per_row / 3).max(1),
                            Duration::from_millis(33),
                        )
                    } else {
                        (
                            opts.rows_per_s,
                            opts.segments_per_row,
                            Duration::from_millis(100),
                        )
                    };
                    builder = RowBuilder::new(opts.fft, rate, rows_per_s, segments);
                }
                Ok(Command::Retune(hz)) => match source.retune(hz) {
                    Ok(true) => {
                        tuning += 1;
                        settle = (SETTLE_S * rate) as u64;
                        sh.rows.lock().unwrap().clear();
                        if rx.is_some() {
                            rx = new_rx(hz, hz, scopes_on).ok();
                            want_service = None;
                            *sh.dab.lock().unwrap() = DabOut::default();
                        }
                    }
                    Ok(false) => {}
                    Err(e) => set_error(e.to_string()),
                },
                Ok(Command::Record(path)) => {
                    if let Some((w, ..)) = writer.take()
                        && let Err(e) = w.finish()
                    {
                        set_error(format!("recording: {e}"));
                    }
                    if let Some(p) = path {
                        match IqFileWriter::create(&p, SampleFormat::Cs8) {
                            Ok(w) => writer = Some((w, p, 0)),
                            Err(e) => set_error(format!("cannot record to {}: {e}", p.display())),
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let n = match meter.time(Stage::Source, || source.read(&mut block)) {
            Ok(n) => n,
            Err(e) => {
                set_error(e.to_string());
                break;
            }
        };
        if n == 0 {
            match source.rewind() {
                Ok(true) => {
                    if let Some(r) = &mut rx {
                        r.demod.reset();
                    }
                    continue;
                }
                _ => {
                    sh.stats.lock().unwrap().ended = true;
                    break;
                }
            }
        }
        // Samples from before a retune took effect are not the new
        // channel's.
        let cut = (settle as usize).min(block.len());
        settle -= cut as u64;
        block.drain(..cut);
        total += cut as u64;
        if block.is_empty() {
            continue;
        }
        let n = block.len();
        total += n as u64;

        level.add(&block);
        if let Some((w, path, bytes)) = &mut writer {
            let r = meter.time(Stage::Record, || w.write(&block));
            *bytes += (n * SampleFormat::Cs8.bytes_per_sample()) as u64;
            if let Err(e) = r {
                set_error(format!("recording to {}: {e}", path.display()));
                writer = None;
            }
        }
        meter.time(Stage::FrontEnd, || front.apply(&mut block));

        // The waterfall.
        let made = meter.time(Stage::Spectrum, || {
            builder.push(&block);
            let mut made = Vec::new();
            while builder.take_row(&mut row).is_some() {
                made.push(Row {
                    db: row.clone(),
                    center_hz: source.center_freq(),
                    sample_rate: rate,
                    tuning,
                });
            }
            made
        });
        if !made.is_empty() {
            let mut q = sh.rows.lock().unwrap();
            q.extend(made);
            let extra = q.len().saturating_sub(MAX_QUEUED_ROWS);
            q.drain(..extra);
        }

        // The receiver.
        if let Some(r) = &mut rx {
            let dec = &mut r.dec;
            meter.time(Stage::Ofdm, || {
                r.fe.process(&block, dec);
                r.demod.push(dec, |f| {
                    frames.push(f.soft.to_vec());
                    frame_stats = Some(f.stats);
                    if let Some(s) = f.scope {
                        new_scope = Some(Arc::new(s.clone()));
                    }
                });
            });
            for soft in frames.drain(..) {
                meter.time(Stage::Fic, || r.fic.push_frame(&soft[..FIC_BITS]));
                // The service asked for, once the FIC describes it.
                if r.service.is_none()
                    && let Some(sid) = want_service
                {
                    let e = &r.fic.ensemble;
                    if let Some(svc) = e.services.get(&sid)
                        && let Some(comp) = svc.primary()
                        && let Some(sc) = e.subchannel_of(comp)
                        && let Some(d) = ServiceDecoder::new(comp, *sc)
                    {
                        r.service = Some((sid, d));
                        if opts.sound {
                            match AudioPlayer::start_pcm(&svc.name(), OutputKind::Device) {
                                Ok(p) => r.player = Some(p),
                                Err(e) => set_error(format!("sound: {e}")),
                            }
                        }
                    }
                }
                if let Some((_, d)) = &mut r.service {
                    meter.time(Stage::Audio, || {
                        for c in 0..CIFS {
                            let a = FIC_BITS + c * CIF_BITS;
                            d.push_cif(&soft[a..a + CIF_BITS], &mut pcm, &mut pad);
                        }
                    });
                    for p in pcm.drain(..) {
                        if let Some(pl) = &mut r.player {
                            let f: Vec<f32> =
                                p.samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
                            // Mono comes out as two channels already (the
                            // AAC decoder is asked for two; MP2 doubles).
                            pl.push_pcm(p.rate, f);
                        }
                    }
                    for ev in pad.drain(..) {
                        match ev {
                            PadEvent::Dls(l) => r.dls = Some(l),
                            PadEvent::DlsClear => r.dls = None,
                            PadEvent::Mot {
                                name, mime, data, ..
                            } => {
                                if mime.starts_with("image/") {
                                    r.slides += 1;
                                    r.slide = Some(Slide {
                                        name,
                                        mime,
                                        data: Arc::new(data),
                                        serial: r.slides,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            // Hand over what is new.
            if publish_at.elapsed() >= publish {
                publish_at = Instant::now();
                let mut o = sh.dab.lock().unwrap();
                let st = &mut o.status;
                st.locked = r.demod.is_locked();
                if let Some(fs) = frame_stats {
                    st.frames = fs.frame;
                    st.mer_db = fs.mer_db;
                    st.cfo_hz = fs.cfo_hz;
                    st.peak_db = fs.peak_db;
                }
                let fs = r.fic.stats;
                st.fibs = fs.fibs;
                st.fibs_ok = fs.fibs_ok;
                if r.fib_mark.2.elapsed() >= Duration::from_secs(1) {
                    let (ok, all) = (fs.fibs_ok - r.fib_mark.0, fs.fibs - r.fib_mark.1);
                    st.fib_ok_ratio = if all > 0 {
                        ok as f32 / all as f32
                    } else {
                        0.0
                    };
                    r.fib_mark = (fs.fibs_ok, fs.fibs, Instant::now());
                }
                st.playing = r.service.as_ref().map(|(s, _)| *s);
                st.service = r.service.as_ref().map(|(_, d)| d.stats.clone());
                st.subchannel = r.service.as_ref().map(|(_, d)| *d.subchannel());
                st.dls = r.dls.clone();
                st.slide = r.slide.clone();
                st.audio = r.player.as_ref().map(|p| p.handle());
                if r.fic.generation != r.fic_gen {
                    r.fic_gen = r.fic.generation;
                    o.ensemble = Some(Arc::new(r.fic.ensemble.clone()));
                }
                if let Some(s) = new_scope.take() {
                    o.scope = Some(s);
                }
            }
        }

        // A file is held to real time.
        if !live {
            let ahead = total as f64 / rate - started.elapsed().as_secs_f64();
            if ahead > 0.02 {
                std::thread::sleep(Duration::from_secs_f64(ahead.min(0.2)));
            }
        }

        if meter.tick() {
            let mut s = sh.stats.lock().unwrap();
            s.samples = total;
            s.center_hz = source.center_freq();
            s.describe = source.describe();
            if level.n > 0 {
                s.rms_dbfs = (10.0 * (level.power / level.n as f64).max(1e-12).log10()) as f32;
                s.peak = level.peak;
                s.clip_percent = (level.clipped as f64 * 100.0 / level.n as f64) as f32;
            }
            level = Level::default();
            if let Some(d) = &drops {
                s.dropped = d();
            }
            s.front_end = front.reading();
            s.load = meter.last;
            s.recording = writer.as_ref().map(|(_, p, b)| Recording {
                path: p.clone(),
                bytes: *b,
            });
        }
    }
}
