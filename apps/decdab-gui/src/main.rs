// made by CasualArclamp
// A release exe is a Windows GUI program: no console window opens with it
// (`attach_console` joins the one that started it, if there is one). Debug
// builds keep theirs, for the log.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! DecDAB desktop GUI, laid out as DecDVB-T's.
//!
//! A HackRF (or an IQ file) in, the spectrum and waterfall over Band III,
//! tuning by block, an IQ recorder and the cost of it all in the status bar;
//! the ensemble's card and services on the right, with the playing service's
//! dynamic label and slideshow; and, at the bottom, the DQPSK scopes and the
//! channel measurements.

mod band;
mod card;
mod channel_view;
mod freq_display;
mod guide;
mod logos;
mod mux_map;
mod prefs;
mod recorder;
mod remember;
mod scopes;
mod sdr;
mod services;
mod sfn_view;
mod source;
mod waterfall;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use decdab_core::channel::{Block, block_at, parse_block, scan_blocks};
use decdab_engine::{Command, CpuGauge, DabStatus, FrontEndSettings, Row, Stats};
use decdab_fic::Ensemble;
use decdab_io::name_hint;
use eframe::egui::{self, Color32, RichText};
use source::{Gains, Session, Spec};
use waterfall::{History, RingImage};

const AMBER: Color32 = Color32::from_rgb(255, 176, 40);
const RED: Color32 = Color32::from_rgb(255, 90, 80);
const GREEN: Color32 = Color32::from_rgb(110, 220, 130);

/// Height of the scopes panel.
const SCOPES_PANEL_H: f32 = 250.0;
/// The display rates the list offers.
const FPS_CHOICES: [f64; 6] = [5.0, 10.0, 15.0, 20.0, 30.0, 60.0];
/// HackRF sample rates: DAB's 2.048 MS/s and its multiples (the front end
/// halves the rate with halfband filters, so no resampler runs).
const RATES_MSPS: [f64; 3] = [2.048, 4.096, 8.192];
/// Half an ensemble plus a margin: received without retuning only if all of
/// this is inside the span.
const ENSEMBLE_HALF_NEEDED_HZ: f64 = 0.85e6;

struct Options {
    file: Option<PathBuf>,
    block: Option<Block>,
    rate_msps: Option<f64>,
    screenshot: Option<PathBuf>,
    /// Record the app's own frames to a video: the file, when to start and
    /// for how long (seconds).
    record_video: Option<PathBuf>,
    record_from: f64,
    record_secs: f64,
    after: f64,
    fps: f64,
    bottom: Bottom,
    decode: bool,
    /// Play this service (its SId in hex, or part of its label) once the FIC
    /// lists it.
    play: Option<String>,
    window: Option<String>,
    /// No sound (a screenshot or a test run).
    mute: bool,
    bottom_given: bool,
    fps_given: bool,
    decode_given: bool,
    kept: remember::Saved,
}

/// What the bottom panel shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bottom {
    Scopes,
    /// The channel sounder: fading, delay–Doppler, Doppler by carrier.
    Channel,
    /// The programme guide (SPI).
    Guide,
    /// The SFN's transmitters (TII) and their delays.
    Sfn,
    Hidden,
}

impl Bottom {
    fn name(self) -> &'static str {
        match self {
            Bottom::Scopes => "scopes",
            Bottom::Channel => "channel",
            Bottom::Guide => "guide",
            Bottom::Sfn => "sfn",
            Bottom::Hidden => "none",
        }
    }

    fn parse(s: &str) -> Option<Bottom> {
        Some(match s {
            "scopes" => Bottom::Scopes,
            "channel" => Bottom::Channel,
            "guide" => Bottom::Guide,
            "sfn" => Bottom::Sfn,
            "none" => Bottom::Hidden,
            _ => return None,
        })
    }
}

impl Options {
    /// A screenshot or a recording: the same every time, so nothing kept
    /// from or for other runs, and no sound.
    fn test_run(&self) -> bool {
        self.screenshot.is_some() || self.record_video.is_some()
    }

    fn from_args() -> Result<Options, String> {
        let mut o = Options {
            file: None,
            block: None,
            rate_msps: None,
            screenshot: None,
            record_video: None,
            record_from: 5.0,
            record_secs: 12.0,
            after: 6.0,
            fps: 15.0,
            bottom: Bottom::Scopes,
            decode: true,
            play: None,
            window: None,
            mute: false,
            bottom_given: false,
            fps_given: false,
            decode_given: false,
            kept: remember::Saved::default(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            let mut value = |what: &str| args.next().ok_or(format!("{a} needs {what}"));
            match a.as_str() {
                "--block" => {
                    let v = value("a block, e.g. 12B")?;
                    o.block = Some(parse_block(&v).ok_or(format!("not a Band III block: {v}"))?);
                }
                "--rate" => {
                    o.rate_msps = Some(value("MS/s")?.parse().map_err(|_| "bad --rate")?);
                }
                "--screenshot" => o.screenshot = Some(value("a .png path")?.into()),
                "--record-video" => o.record_video = Some(value("an .mp4 path")?.into()),
                "--record-from" => {
                    o.record_from = value("seconds")?.parse().map_err(|_| "bad --record-from")?
                }
                "--record-secs" => {
                    o.record_secs = value("seconds")?.parse().map_err(|_| "bad --record-secs")?
                }
                "--bottom" => {
                    let v = value("scopes, channel or none")?;
                    o.bottom =
                        Bottom::parse(&v).ok_or(format!("--bottom: not scopes, channel, guide, sfn or none: {v}"))?;
                    o.bottom_given = true;
                }
                "--window" => o.window = Some(value("sdr")?),
                "--play" => o.play = Some(value("a service (SId or label)")?),
                "--mute" => o.mute = true,
                "--decode" | "--no-decode" => {
                    o.decode = a == "--decode";
                    o.decode_given = true;
                }
                "--fps" => {
                    o.fps = value("frames a second")?.parse().map_err(|_| "bad --fps")?;
                    o.fps_given = true;
                }
                "--after" => o.after = value("seconds")?.parse().map_err(|_| "bad --after")?,
                f if !f.starts_with("--") => o.file = Some(f.into()),
                _ => return Err(format!("unknown option {a}")),
            }
        }
        let kept = remember::Saved::load();
        if !o.test_run() {
            if !o.bottom_given
                && let Some(b) = kept.bottom.as_deref().and_then(Bottom::parse)
            {
                o.bottom = b;
            }
            if !o.fps_given
                && let Some(f) = kept.fps.filter(|f| FPS_CHOICES.contains(f))
            {
                o.fps = f;
            }
            if !o.decode_given
                && let Some(d) = kept.decode
            {
                o.decode = d;
            }
        } else {
            o.mute = true;
        }
        o.kept = kept;
        Ok(o)
    }
}

/// Started from a terminal, join its console (see DecDVB-T).
#[cfg(all(windows, not(debug_assertions)))]
fn attach_console() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };
    // SAFETY: plain Win32 calls with valid arguments; the handles given to
    // `SetStdHandle` are leaked on purpose, as the standard handles are for
    // the life of the process.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let h = GetStdHandle(which);
            if (h.is_null() || h as isize == -1)
                && let Ok(f) = std::fs::OpenOptions::new().write(true).open("CONOUT$")
            {
                SetStdHandle(which, f.into_raw_handle() as _);
            }
        }
    }
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_console() {}

/// The window's icon, from the PNG built in.
fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon-256.png")).ok()?;
    let rgba = img.to_rgba8();
    Some(egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn main() -> eframe::Result {
    attach_console();
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("decdab-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let opts = match Options::from_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("decdab-gui: {e}");
            eprintln!(
                "usage: decdab-gui [capture.cs8] [--block 12B] [--rate MSPS] [--play SERVICE] \
                 [--screenshot OUT.png] [--after SECS] [--fps N] [--bottom scopes|none]                  [--record-video OUT.mp4 --record-from SECS --record-secs SECS] \
                 [--window sdr] [--mute] [--no-decode]"
            );
            std::process::exit(2);
        }
    };
    let (w, h) = opts
        .kept
        .window
        .filter(|_| !opts.test_run())
        .unwrap_or((1400, 860));
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([w as f32, h as f32])
        .with_min_inner_size([900.0, 560.0])
        .with_title("DecDAB")
        .with_drag_and_drop(true);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "DecDAB",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            cc.egui_ctx.style_mut_of(egui::Theme::Dark, |s| {
                s.override_text_style = Some(egui::TextStyle::Monospace);
            });
            Ok(Box::new(App::new(opts)))
        }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    HackRf,
    File,
}

struct App {
    opts: Options,
    bottom: Bottom,
    scope: scopes::ScopeView,
    scopes_sent: bool,
    /// The channel panel, and whether the engine was last told to sound.
    chan_view: channel_view::ChannelView,
    sounder_sent: bool,
    /// The SFN panel, and whether the engine was last told to look for TII.
    sfn_view: sfn_view::SfnView,
    tii_sent: bool,
    started: Instant,
    shot_sent: bool,
    cpu_mark: Option<(f64, Instant)>,
    cpu_avg: Option<f32>,
    frame_cost: [Duration; 2],
    frames: u32,
    last_frame: Option<Instant>,
    delay_ms: f64,

    session: Option<Session>,
    kind: SourceKind,
    /// The block being received (the VFO).
    block: Block,
    block_text: String,
    rate_msps: f64,
    gains: Gains,
    file: Option<PathBuf>,
    file_centre_mhz: f64,
    file_rate_msps: f64,

    centre_hz: f64,
    span_hz: f64,
    vfo_hz: Option<f64>,
    tuning: u32,
    smooth: Vec<f32>,
    history: History,
    ring: RingImage,

    stats: Stats,
    stats_at: Instant,
    cpu: CpuGauge,
    cpu_percent: f32,
    note: String,

    /// The receiver: on or off, the frequency it was last asked to decode,
    /// what it reports, the ensemble, the service picked and the one asked
    /// for.
    decode: bool,
    decoder_sent: Option<Option<f64>>,
    dab: DabStatus,
    ensemble: Arc<Ensemble>,
    selected: Option<u32>,
    play_sent: Option<Option<u32>>,
    want_play: Option<u32>,
    /// The slideshow's picture as a texture, with the slide it came from.
    slide_tex: Option<(u64, egui::TextureHandle)>,
    /// Station logos and the guide, from the ensemble's SPI service.
    logos: logos::Logos,
    guide: guide::GuideView,
    volume: f32,
    /// MP2 error concealment: the user's choice (kept in the prefs) and
    /// whether the engine has been told it.
    mp2_conceal: bool,
    conceal_sent: bool,

    tune_hz: f64,
    dial_hz: f64,
    dial_edit: Option<Instant>,
    sdr: sdr::SdrSettings,
    sdr_sent: Option<sdr::SdrSettings>,
    sdr_open: bool,
    optimiser: Option<sdr::Optimiser>,
    opt_result: Option<String>,
    /// The display rate the engine was last told (it makes a waterfall row
    /// a frame), and the rows come but not yet shown: one goes on the
    /// waterfall each frame, so it scrolls evenly.
    fps_sent: Option<f64>,
    /// The waterfall's height on the screen, physical pixels: its rows, one
    /// a pixel, so it moves a pixel a frame and the frame rate is its speed.
    wf_rows: usize,
    /// Frames that put 0, 1 and 2 rows on the waterfall (for the screenshot
    /// report: an even scroll is all 1s).
    wf_steps: [u32; 3],
    rows_due: std::collections::VecDeque<Row>,
    keeper: Option<remember::Keeper>,
    recorder: Option<recorder::Recorder>,
    radio: (Block, f64),
    window_size: (u32, u32),
}

impl App {
    fn new(mut opts: Options) -> App {
        let kept = std::mem::take(&mut opts.kept);
        let by_hand = opts.block.is_some() || opts.file.is_some();
        let mut block = opts
            .block
            .or(kept.block.as_deref().and_then(parse_block))
            .unwrap_or(parse_block("12B").expect("12B is a block"));
        let mut tune_hz = block.hz();
        if !by_hand && let Some(h) = kept.tune_hz {
            tune_hz = h;
            block = block_at(h).unwrap_or(block);
        }
        let file = kept.file.clone().filter(|f| f.exists());
        let kind = if !by_hand && kept.file_source && file.is_some() {
            SourceKind::File
        } else {
            SourceKind::HackRf
        };
        let volume = prefs::volume();
        decdab_audio::set_volume(volume);
        let mut app = App {
            started: Instant::now(),
            shot_sent: false,
            cpu_mark: None,
            cpu_avg: None,
            frame_cost: [Duration::ZERO; 2],
            frames: 0,
            last_frame: None,
            delay_ms: 1000.0 / 15.0,
            session: None,
            kind,
            block,
            block_text: block.name.to_string(),
            rate_msps: opts
                .rate_msps
                .or(kept.rate_msps.filter(|r| RATES_MSPS.contains(r)))
                .unwrap_or(4.096),
            gains: kept.gains.unwrap_or_default(),
            file,
            file_centre_mhz: kept.file_centre_mhz.unwrap_or(0.0),
            file_rate_msps: kept.file_rate_msps.unwrap_or(4.096),
            centre_hz: 0.0,
            span_hz: 0.0,
            vfo_hz: Some(tune_hz),
            tuning: 0,
            smooth: Vec::new(),
            history: History::default(),
            ring: RingImage::default(),
            bottom: opts.bottom,
            scope: scopes::ScopeView::default(),
            scopes_sent: false,
            chan_view: channel_view::ChannelView::default(),
            sounder_sent: false,
            sfn_view: sfn_view::SfnView::default(),
            tii_sent: false,
            stats: Stats::default(),
            stats_at: Instant::now(),
            cpu: CpuGauge::default(),
            cpu_percent: 0.0,
            note: String::new(),
            decode: opts.decode,
            decoder_sent: None,
            dab: DabStatus::default(),
            ensemble: Arc::new(Ensemble::default()),
            selected: None,
            play_sent: None,
            want_play: None,
            slide_tex: None,
            logos: logos::Logos::default(),
            guide: guide::GuideView::default(),
            volume,
            mp2_conceal: prefs::get("mp2_conceal").is_some_and(|v| v == "1"),
            conceal_sent: false,
            tune_hz,
            dial_hz: tune_hz,
            dial_edit: None,
            sdr: sdr::SdrSettings::load(),
            sdr_sent: None,
            sdr_open: opts.window.as_deref() == Some("sdr"),
            optimiser: None,
            opt_result: None,
            fps_sent: None,
            wf_rows: 0,
            wf_steps: [0; 3],
            rows_due: std::collections::VecDeque::new(),
            keeper: None,
            recorder: None,
            radio: (block, tune_hz),
            window_size: kept.window.unwrap_or((1400, 860)),
            opts,
        };
        if let Some(f) = app.opts.file.clone() {
            app.open_file(f);
        } else if app.opts.block.is_some() {
            app.start();
        }
        if let Some(p) = app.opts.record_video.clone() {
            app.recorder = Some(recorder::Recorder::new(
                p,
                app.opts.record_from,
                app.opts.record_secs,
                app.opts.fps,
            ));
        }
        if !app.opts.test_run() {
            app.keeper = Some(remember::Keeper::new());
        }
        app
    }

    fn snapshot(&self) -> remember::Snapshot {
        let (block, tune_hz) = if self.kind == SourceKind::HackRf {
            (self.block, self.tune_hz)
        } else {
            self.radio
        };
        remember::Snapshot {
            block: block.name.to_string(),
            tune_hz: tune_hz.round() as i64,
            file_source: self.kind == SourceKind::File,
            rate_khz: (self.rate_msps * 1e3).round() as i64,
            gains: self.gains,
            file: self
                .file
                .as_ref()
                .map_or(String::new(), |p| p.display().to_string()),
            file_centre_hz: (self.file_centre_mhz * 1e6).round() as i64,
            file_rate_khz: (self.file_rate_msps * 1e3).round() as i64,
            decode: self.decode,
            bottom: self.bottom.name().to_string(),
            fps: self.opts.fps as u32,
            doppler_hz: 0,
            window: self.window_size,
        }
    }

    fn remember(&mut self, ctx: &egui::Context) {
        if self.keeper.is_none() {
            return;
        }
        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            self.window_size = (r.width().round() as u32, r.height().round() as u32);
        }
        if self.kind == SourceKind::HackRf {
            self.radio = (self.block, self.tune_hz);
        }
        let now = self.snapshot();
        if let Some(k) = self.keeper.as_mut()
            && let Some(after) = k.look(now, Instant::now())
        {
            ctx.request_repaint_after(after);
        }
    }

    // ---- sources --------------------------------------------------------

    fn start(&mut self) {
        self.stop();
        match self.kind {
            SourceKind::HackRf => {
                let fc = self.tune_hz;
                if fc <= 0.0 {
                    return;
                }
                let spec = Spec::HackRf {
                    center_hz: fc,
                    rate_hz: self.rate_msps * 1e6,
                    gains: self.gains,
                };
                self.begin(spec, Some(fc));
            }
            SourceKind::File => {
                let Some(path) = self.file.clone() else {
                    self.open_file_dialog();
                    return;
                };
                let spec = Spec::File {
                    path,
                    center_hz: self.file_centre_mhz * 1e6,
                    rate_hz: self.file_rate_msps * 1e6,
                };
                let c = self.file_centre_mhz * 1e6;
                let vfo = block_at(c).map(|b| b.hz()).or((c > 0.0).then_some(c));
                self.begin(spec, vfo);
            }
        }
    }

    fn begin(&mut self, spec: Spec, vfo: Option<f64>) {
        match Session::open(&spec, !self.opts.mute) {
            Ok(s) => {
                self.session = Some(s);
                self.decoder_sent = None;
                self.reset_ensemble();
                self.vfo_hz = vfo;
                self.reset_view();
                self.note.clear();
                (self.scopes_sent, self.sdr_sent, self.fps_sent) = (false, None, None);
                self.sounder_sent = false;
                self.tii_sent = false;
                self.conceal_sent = false;
                self.rows_due.clear();
                self.optimiser = None;
            }
            Err(e) => self.note = e,
        }
    }

    fn stop(&mut self) {
        self.session = None;
    }

    fn open_file_dialog(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("IQ capture", &["cs8", "cu8", "cs16", "cf32", "wav"])
            .pick_file()
        {
            self.open_file(p);
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        let hint = name_hint(&path);
        self.kind = SourceKind::File;
        self.file_centre_mhz = hint.center_hz.map_or(0.0, |h| h / 1e6);
        self.file_rate_msps = hint.sample_rate.map_or(2.048, |r| r / 1e6);
        self.file = Some(path);
        self.start();
    }

    fn reset_view(&mut self) {
        self.history.reset();
        self.rows_due.clear();
        self.smooth.clear();
        self.span_hz = 0.0;
        self.scope.reset();
        self.chan_view.reset();
    }

    /// A different ensemble (or none): its services, label and picture go.
    fn reset_ensemble(&mut self) {
        self.dab = DabStatus::default();
        self.ensemble = Arc::new(Ensemble::default());
        self.selected = None;
        self.play_sent = None;
        self.want_play = None;
        self.slide_tex = None;
        self.logos.clear();
        self.guide = guide::GuideView::default();
    }

    /// Ask the engine to decode the ensemble under the VFO while the decode
    /// switch is on, and to stop when it is not.
    fn sync_decoder(&mut self) {
        let Some(s) = &self.session else { return };
        let want = self
            .decode
            .then(|| {
                self.vfo_hz
                    .or((self.centre_hz > 0.0).then_some(self.centre_hz))
            })
            .flatten();
        if self.decoder_sent != Some(want) && (want.is_some() || self.decoder_sent.is_some()) {
            s.rx.send(Command::Decode(want));
            self.decoder_sent = Some(want);
            self.reset_ensemble();
        }
    }

    fn sync_play(&mut self) {
        let Some(s) = &self.session else { return };
        // A service asked for on the command line plays once it is listed.
        if let Some(pat) = &self.opts.play
            && let Some(sid) = find_service(&self.ensemble, pat)
        {
            self.opts.play = None;
            self.want_play = Some(sid);
            self.selected = Some(sid);
        }
        if self.play_sent != Some(self.want_play) {
            s.rx.send(Command::Select(self.want_play));
            self.play_sent = Some(self.want_play);
            self.slide_tex = None;
        }
    }

    /// The MP2 decoder's error concealment follows the checkbox.
    fn sync_conceal(&mut self) {
        let Some(s) = &self.session else { return };
        if self.mp2_conceal != self.conceal_sent {
            s.rx.send(Command::Mp2Conceal(self.mp2_conceal));
            self.conceal_sent = self.mp2_conceal;
        }
    }

    /// The sounder runs only while its panel shows.
    fn sync_sounder(&mut self) {
        let Some(s) = &self.session else { return };
        let want = self.bottom == Bottom::Channel && self.decode;
        if want != self.sounder_sent {
            s.rx.send(Command::Sounder(want));
            self.sounder_sent = want;
            if !want {
                self.chan_view.reset();
            }
        }
    }

    /// TII is looked for only while the SFN panel shows.
    fn sync_tii(&mut self) {
        let Some(s) = &self.session else { return };
        let want = self.bottom == Bottom::Sfn && self.decode;
        if want != self.tii_sent {
            s.rx.send(Command::Tii(want));
            self.tii_sent = want;
            if !want {
                self.sfn_view.reset();
            }
        }
    }

    fn sync_scopes(&mut self) {
        let Some(s) = &self.session else { return };
        let want = self.bottom == Bottom::Scopes && self.decode;
        if want != self.scopes_sent {
            s.rx.send(Command::Scopes(want));
            self.scopes_sent = want;
            if !want {
                self.scope.reset();
            }
        }
    }

    // ---- tuning ---------------------------------------------------------

    /// Receive block `b`: move the VFO if the span already holds it, retune
    /// the radio if not.
    fn tune_block(&mut self, b: Block) {
        self.cancel_optimiser();
        self.block = b;
        self.block_text = b.name.to_string();
        self.tune_hz = b.hz();
        self.set_vfo(b.hz());
    }

    fn set_vfo(&mut self, hz: f64) {
        let fits = self.span_hz > 0.0
            && (hz - self.centre_hz).abs() + ENSEMBLE_HALF_NEEDED_HZ <= self.span_hz / 2.0;
        match &self.session {
            Some(s) if s.live && !fits => {
                s.rx.send(Command::Retune(hz));
                self.centre_hz = hz;
                self.reset_view();
                self.reset_ensemble();
                self.vfo_hz = Some(hz);
            }
            Some(s) if !s.live && !fits => {
                self.note = format!("{:.3} MHz is outside this capture", hz / 1e6);
            }
            _ => {
                self.vfo_hz = Some(hz);
                self.reset_ensemble();
            }
        }
    }

    fn step_block(&mut self, up: bool) {
        let list: Vec<Block> = scan_blocks().collect();
        let at = list
            .iter()
            .position(|b| b.khz >= self.block.khz)
            .unwrap_or(0);
        let next = if up {
            list[(at + usize::from(list[at].khz == self.block.khz)).min(list.len() - 1)]
        } else {
            list[at.saturating_sub(1)]
        };
        self.tune_block(next);
    }

    fn set_frequency(&mut self, hz: f64) {
        self.cancel_optimiser();
        self.tune_hz = hz;
        if let Some(b) = block_at(hz) {
            self.block = b;
            self.block_text = b.name.to_string();
        }
        match &self.session {
            Some(s) if s.live => {
                s.rx.send(Command::Retune(hz));
                self.centre_hz = hz;
                self.reset_view();
                self.reset_ensemble();
                self.vfo_hz = Some(hz);
            }
            Some(_) => self.set_vfo(hz),
            None => self.vfo_hz = Some(hz),
        }
    }

    fn freq_dial(&mut self, ui: &mut egui::Ui) {
        if self.dial_edit.is_none() {
            self.dial_hz = self.vfo_hz.unwrap_or(self.tune_hz);
        }
        if freq_display::show(ui, &mut self.dial_hz, 1e6, 6e9, 34.0) {
            self.dial_edit = Some(Instant::now());
        }
    }

    fn apply_dial(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.dial_edit {
            if t.elapsed() >= Duration::from_millis(150) {
                self.dial_edit = None;
                self.set_frequency(self.dial_hz);
            } else {
                ctx.request_repaint_after(Duration::from_millis(160));
            }
        }
    }

    fn sync_sdr(&mut self) {
        let Some(s) = &self.session else { return };
        if self.sdr_sent == Some(self.sdr) {
            return;
        }
        let first = self.sdr_sent.is_none();
        s.rx.send(Command::FrontEnd(FrontEndSettings {
            dc_removal: self.sdr.dc_removal,
            iq_balance: self.sdr.iq_balance,
        }));
        let mut errors = Vec::new();
        if (!first || self.sdr.filter_mhz.is_some())
            && let Err(e) = s.set_filter_mhz(self.sdr.filter_mhz)
        {
            errors.push(e);
        }
        if (!first || self.sdr.ppm != 0.0)
            && let Err(e) = s.set_ppm(self.sdr.ppm)
        {
            errors.push(e);
        }
        if let Some(e) = errors.pop() {
            self.note = e;
        }
        self.sdr_sent = Some(self.sdr);
        self.sdr.save();
    }

    /// The waterfall's row rate follows the display rate: a row a frame.
    fn sync_fps(&mut self) {
        let fps = self.opts.fps;
        let Some(s) = &self.session else { return };
        if self.fps_sent != Some(fps) {
            s.rx.send(Command::DisplayFps(fps));
            self.fps_sent = Some(fps);
        }
        // Exactly a row a physical pixel of the waterfall's height: a row a
        // frame then moves it one whole pixel a frame, 60 pixels a second at
        // 60 fps, 15 at 15 (the time on the screen is the height over the
        // rate). Any other count would draw rows across fractions of pixels.
        // A resize or a change of scaling starts the waterfall afresh.
        if self.wf_rows > 0 && self.history.capacity() != self.wf_rows {
            self.history.set_capacity(self.wf_rows);
        }
    }

    fn start_optimiser(&mut self) {
        if !self.session.as_ref().is_some_and(|s| s.live) {
            self.note = "Optimising the gain needs a live radio".into();
            return;
        }
        self.opt_result = None;
        self.optimiser = Some(sdr::Optimiser::new(self.gains));
    }

    fn cancel_optimiser(&mut self) {
        if let Some(o) = self.optimiser.take() {
            self.gains = o.original;
            self.change_gains();
            self.opt_result = Some("cancelled".into());
        }
    }

    fn optimise_tick(&mut self) {
        let Some(o) = self.optimiser.as_mut() else {
            return;
        };
        if self.session.is_none() {
            self.optimiser = None;
            return;
        }
        let reading = sdr::Reading {
            now: Instant::now(),
            mer: (self.dab.locked && self.dab.mer_db.is_finite()).then_some(self.dab.mer_db),
            cn: None,
            rms_dbfs: self.stats.rms_dbfs,
            clip_percent: self.stats.clip_percent,
        };
        match o.tick(&reading) {
            sdr::Action::Wait => {}
            sdr::Action::Apply(g) => {
                self.gains = g;
                self.change_gains();
            }
            sdr::Action::Finish(best) => {
                let o = self.optimiser.take().expect("running");
                let found = best.zip(o.best());
                self.gains = best.unwrap_or(o.original);
                self.change_gains();
                self.opt_result = Some(match found {
                    Some((g, p)) => format!(
                        "best: LNA {} VGA {}, {:.1} dB MER (level {:.0} dBFS), {} settings tried",
                        g.lna,
                        g.vga,
                        p.mer.or(p.cn).unwrap_or(0.0),
                        p.rms_dbfs,
                        o.done()
                    ),
                    None => "nothing to measure (no lock, or clipping at every setting)".into(),
                });
            }
        }
    }

    fn change_gains(&mut self) {
        if let Some(s) = &self.session
            && let Err(e) = s.set_gains(self.gains)
        {
            self.note = e;
        }
    }

    // ---- recording ------------------------------------------------------

    fn recording(&self) -> bool {
        self.stats.recording.is_some()
    }

    fn toggle_record(&mut self) {
        let Some(s) = &self.session else { return };
        if self.recording() {
            s.rx.send(Command::Record(None));
            self.stats.recording = None;
            return;
        }
        let dir = PathBuf::from("captures");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.note = format!("cannot make {}: {e}", dir.display());
            return;
        }
        let name = format!(
            "{}_{:.3}MHz_{}Msps{}_{}.cs8",
            block_at(self.centre_hz).map_or("f".to_string(), |b| b.name.to_ascii_lowercase()),
            self.centre_hz / 1e6,
            self.stats.sample_rate / 1e6,
            if self.gains.amp { "_amp" } else { "" },
            utc_stamp()
        );
        let path = dir.join(name);
        self.note = format!("recording to {}", path.display());
        s.rx.send(Command::Record(Some(path)));
    }

    // ---- per-frame work -------------------------------------------------

    fn take_rows(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.session else { return };
        let rows: Vec<Row> = s.rx.take_rows();
        let new_stats =
            (self.stats_at.elapsed() >= Duration::from_millis(250)).then(|| s.rx.stats());
        if self.decode {
            let mut u = s.rx.take_dab();
            if self.bottom == Bottom::Channel {
                self.chan_view.update(&mut u);
            }
            self.dab = u.status;
            if let Some(e) = u.ensemble {
                self.ensemble = e;
            }
            if let Some(spi) = u.spi {
                self.logos.update(spi);
            }
            if self.bottom == Bottom::Scopes {
                self.scope.update(u.scope);
            }
            if self.bottom == Bottom::Sfn {
                self.sfn_view.update(u.tii);
            }
            // A new slide: decode it into a texture.
            if let Some(sl) = &self.dab.slide
                && self.slide_tex.as_ref().is_none_or(|(n, _)| *n != sl.serial)
            {
                match image::load_from_memory(&sl.data) {
                    Ok(img) => {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let ci = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        let tex = ctx.load_texture("slide", ci, egui::TextureOptions::LINEAR);
                        self.slide_tex = Some((sl.serial, tex));
                    }
                    Err(e) => {
                        self.note = format!("slide {}: {e}", sl.name);
                        self.slide_tex = None;
                    }
                }
            }
        }
        // The rows wait in a queue and one goes on each frame, so the
        // waterfall moves a row a frame even when the engine's rows and the
        // frames do not fall in step; a queue that grows (the display slower
        // than asked) is caught up two rows a frame, and one that has run
        // far behind is cut.
        self.rows_due.extend(rows);
        let backlog = self.rows_due.len();
        if backlog > (self.opts.fps as usize).max(10) {
            self.rows_due.drain(..backlog - 2);
        }
        let take = match self.rows_due.len() {
            0 => 0,
            n if n > 3 => 2,
            _ => 1,
        };
        // The trace's smoothing: a time constant of about 0.12 s, whatever
        // the row rate.
        let pull = 1.0 - (-1.0 / (0.12 * self.opts.fps.max(1.0)) as f32).exp();
        if self.span_hz > 0.0 && self.decoder_sent.is_some() {
            self.wf_steps[take.min(2)] += 1;
        }
        for row in self.rows_due.drain(..take).collect::<Vec<_>>() {
            if row.tuning != self.tuning {
                self.tuning = row.tuning;
                self.reset_view();
            }
            self.centre_hz = row.center_hz;
            self.span_hz = row.sample_rate;
            if self.smooth.len() != row.db.len() {
                self.smooth.clone_from(&row.db);
            } else {
                for (s, v) in self.smooth.iter_mut().zip(&row.db) {
                    *s += pull * (v - *s);
                }
            }
            self.history.push(row.db);
        }
        if let Some(stats) = new_stats {
            self.stats = stats;
            self.stats_at = Instant::now();
            if let Some(e) = &self.stats.error {
                self.note.clone_from(e);
            }
        }
    }

    fn pace(&mut self) -> Duration {
        let target = 1000.0 / self.opts.fps.clamp(1.0, 60.0);
        let now = Instant::now();
        if let Some(prev) = self.last_frame.replace(now) {
            let gap = now.duration_since(prev).as_secs_f64() * 1e3;
            if gap < 4.0 * target {
                self.delay_ms += 0.5 * (target - gap);
            }
        }
        self.delay_ms = self.delay_ms.clamp(1.0, 4.0 * target);
        Duration::from_secs_f64(self.delay_ms / 1e3)
    }

    fn card(&self) -> card::Card {
        let d = &self.dab;
        let e = &self.ensemble;
        let locked = self.decode && d.locked && d.frames > 0;
        let mut details = Vec::new();
        let kv = |k: &str, v: String| (k.to_string(), v);
        // The code the margin is for: the playing service's, else the FIC's.
        let (rate, rate_name) = match d.subchannel.as_ref().and_then(|s| Some((s, s.plan()?))) {
            Some((s, p)) => (p.code_rate(), s.protection_name()),
            None => (decdab_fec::Plan::fic().code_rate(), "FIC".to_string()),
        };
        if locked {
            details.push(kv("ECC", e.ecc.map_or("–".into(), |v| format!("{v:02X}"))));
            details.push(kv(
                "code rate",
                format!("{}/{} · {rate_name}", rate.0, rate.1),
            ));
            details.push(kv(
                "EId",
                e.eid.map_or("–".into(), |v| format!("{v:04X}")),
            ));
            details.push(kv("services", e.services.len().to_string()));
            details.push(kv("sub-channels", e.subchannels.len().to_string()));
            details.push(kv("FIBs good", format!("{:.0} %", d.fib_ok_ratio * 100.0)));
            details.push(kv("carrier", format!("{:+.1} Hz", d.cfo_hz)));
            if let Some(t) = e.time {
                let (y, m, dd) = t.date();
                details.push(kv(
                    "time (UTC)",
                    format!("{y:04}-{m:02}-{dd:02} {:02}:{:02}", t.hours, t.minutes),
                ));
            }
        }
        card::Card {
            standard: locked.then_some(card::Standard::Dab),
            assumed: false,
            name: locked.then(|| e.name()).filter(|n| !n.is_empty()),
            qef_db: locked.then(|| card::required_mer_db(rate.0 as f32 / rate.1 as f32)),
            details,
            cn_db: locked.then_some(d.mer_db),
            cn_source: "MER",
            channel: self.vfo_hz.and_then(block_at).map(|b| b.name.to_string()),
            freq_hz: self.vfo_hz,
            active: self.session.is_some(),
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.opts.screenshot.clone() else {
            return;
        };
        if self.cpu_mark.is_none() && self.started.elapsed().as_secs_f64() >= 2.0 {
            self.cpu_mark = Some((decdab_engine::process_cpu_seconds(), Instant::now()));
        }
        if !self.shot_sent && self.started.elapsed().as_secs_f64() >= self.opts.after {
            if let Some((c0, t0)) = self.cpu_mark {
                let dt = t0.elapsed().as_secs_f64();
                self.cpu_avg =
                    Some((100.0 * (decdab_engine::process_cpu_seconds() - c0) / dt) as f32);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.shot_sent = true;
        }
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        let Some(image) = image else { return };
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
        match image::save_buffer(&path, &rgba, w, h, image::ColorType::Rgba8) {
            Ok(()) => eprintln!(
                "screenshot saved to {} (CPU {:.1} % average; per frame: logic {:.2} ms, ui {:.2} ms, {} frames; waterfall rows a frame 0/1/2: {:?}; {})",
                path.display(),
                self.cpu_avg.unwrap_or(self.cpu_percent),
                self.frame_cost[0].as_secs_f64() * 1e3 / self.frames.max(1) as f64,
                self.frame_cost[1].as_secs_f64() * 1e3 / self.frames.max(1) as f64,
                self.frames,
                self.wf_steps,
                self.stats.describe
            ),
            Err(e) => eprintln!("cannot save screenshot {}: {e}", path.display()),
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // ---- panels ---------------------------------------------------------

    fn control_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            self.freq_dial(ui);
            ui.add_space(10.0);
            ui.vertical(|ui| self.control_rows(ui));
        });
        ui.add_space(2.0);
    }

    fn control_rows(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let running = self.session.is_some();
            let live = self.kind == SourceKind::HackRf;

            let before = self.kind;
            egui::ComboBox::from_id_salt("source")
                .selected_text(if live { "HackRF" } else { "IQ file" })
                .width(80.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.kind, SourceKind::HackRf, "HackRF");
                    ui.selectable_value(&mut self.kind, SourceKind::File, "IQ file");
                });
            if before != self.kind && running {
                self.stop();
            }

            if self.kind == SourceKind::HackRf {
                ui.label("block");
                if ui.small_button("◀").clicked() {
                    self.step_block(false);
                }
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.block_text)
                        .desired_width(44.0)
                        .hint_text("12B"),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    match parse_block(&self.block_text) {
                        Some(b) => self.tune_block(b),
                        None => {
                            self.note = format!("not a Band III block (5A–13F): {}", self.block_text)
                        }
                    }
                    self.block_text = self.block.name.to_string();
                }
                if ui.small_button("▶").clicked() {
                    self.step_block(true);
                }
                ui.label(format!("{:.3} MHz", self.block.hz() / 1e6));

                ui.separator();
                ui.label("rate");
                let before = self.rate_msps;
                egui::ComboBox::from_id_salt("rate")
                    .selected_text(format!("{} MS/s", self.rate_msps))
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for r in RATES_MSPS {
                            ui.selectable_value(&mut self.rate_msps, r, format!("{r} MS/s"));
                        }
                    });
                if before != self.rate_msps && running {
                    self.start();
                }

                ui.separator();
                let mut g = self.gains;
                ui.label("LNA");
                ui.add(egui::DragValue::new(&mut g.lna).range(0..=40).speed(0.2));
                ui.label("VGA");
                ui.add(egui::DragValue::new(&mut g.vga).range(0..=62).speed(0.2));
                ui.checkbox(&mut g.amp, "amp");
                g.lna = g.lna / 8 * 8;
                g.vga = g.vga / 2 * 2;
                if g != self.gains {
                    self.gains = g;
                    self.change_gains();
                }
            } else {
                match &self.file {
                    Some(p) => ui.label(
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ),
                    None => ui.label(RichText::new("no file").weak()),
                };
                ui.label("centre");
                ui.add(
                    egui::DragValue::new(&mut self.file_centre_mhz)
                        .speed(0.1)
                        .suffix(" MHz"),
                );
                ui.label("rate");
                ui.add(
                    egui::DragValue::new(&mut self.file_rate_msps)
                        .speed(0.05)
                        .suffix(" MS/s"),
                );
                if ui.button("Open…").clicked() {
                    self.open_file_dialog();
                }
            }

            ui.separator();
            if running {
                if ui.button(RichText::new("■ Stop").color(RED)).clicked() {
                    self.stop();
                }
            } else if ui.button(RichText::new("▶ Start").color(GREEN)).clicked() {
                self.start();
            }
            if running && live {
                let label = if self.recording() {
                    RichText::new("● Recording").color(RED)
                } else {
                    RichText::new("● Record")
                };
                if ui.button(label).clicked() {
                    self.toggle_record();
                }
            }
            ui.separator();
            if ui
                .selectable_label(self.sdr_open, "⚙ SDR")
                .on_hover_text("DC removal, I/Q balance, filter, frequency correction, gain optimiser")
                .clicked()
            {
                self.sdr_open = !self.sdr_open;
            }
        });
        ui.horizontal(|ui| {
            let s = &self.stats;
            let running = self.session.is_some();
            let level = ((s.rms_dbfs + 60.0) / 60.0).clamp(0.0, 1.0);
            ui.add(
                egui::ProgressBar::new(if running { level } else { 0.0 })
                    .desired_width(180.0)
                    .text(if running {
                        format!("{:.1} dBFS", s.rms_dbfs)
                    } else {
                        "level".into()
                    }),
            );
            if running && s.clip_percent > 0.05 {
                ui.label(RichText::new("OVERLOAD: lower the gain").color(RED));
            } else if running && s.rms_dbfs < -45.0 {
                ui.label(RichText::new("weak: raise the gain").color(AMBER));
            }
            ui.separator();
            ui.checkbox(&mut self.decode, "decode");
            ui.label("panel");
            ui.selectable_value(&mut self.bottom, Bottom::Scopes, "scopes");
            ui.selectable_value(&mut self.bottom, Bottom::Channel, "channel");
            ui.selectable_value(&mut self.bottom, Bottom::Guide, "guide");
            ui.selectable_value(&mut self.bottom, Bottom::Sfn, "SFN");
            ui.selectable_value(&mut self.bottom, Bottom::Hidden, "none");
            if self.bottom == Bottom::Channel {
                egui::ComboBox::from_id_salt("doppler")
                    .selected_text(format!("±{} Hz", self.chan_view.doppler_view))
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for d in channel_view::DOPPLER_VIEWS {
                            ui.selectable_value(
                                &mut self.chan_view.doppler_view,
                                d,
                                format!("±{d} Hz"),
                            );
                        }
                    });
            }
            ui.separator();
            ui.label("display");
            egui::ComboBox::from_id_salt("fps")
                .selected_text(format!("{} fps", self.opts.fps))
                .width(70.0)
                .show_ui(ui, |ui| {
                    // The waterfall scrolls a row a frame: the rate is also
                    // its speed.
                    for f in FPS_CHOICES {
                        ui.selectable_value(&mut self.opts.fps, f, format!("{f} fps"));
                    }
                });
            if !self.note.is_empty() {
                ui.label(RichText::new(&self.note).color(AMBER));
            }
        });
        ui.add_space(2.0);
    }

    fn status_panel(&mut self, ui: &mut egui::Ui) {
        let s = &self.stats;
        let running = self.session.is_some();
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            let grid = |ui: &mut egui::Ui, id: &str, rows: Vec<(String, String)>| {
                egui::Grid::new(id)
                    .num_columns(2)
                    .spacing([14.0, 2.0])
                    .show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(RichText::new(k).weak());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
            };
            let kv = |k: &str, v: String| (k.to_string(), v);
            grid(
                ui,
                "receiver",
                vec![
                    kv(
                        "state",
                        match (&self.session, s.ended) {
                            (None, _) => "stopped".into(),
                            (Some(_), true) => "end of file".into(),
                            _ if self.decode && self.dab.locked => "locked".into(),
                            _ if self.decode => "searching".into(),
                            _ => "running".into(),
                        },
                    ),
                    kv("source", s.describe.clone()),
                    kv(
                        "centre / span",
                        format!(
                            "{:.3} MHz / {:.3} MHz",
                            s.center_hz / 1e6,
                            s.sample_rate / 1e6
                        ),
                    ),
                    kv(
                        "block",
                        self.vfo_hz
                            .and_then(block_at)
                            .map_or("—".into(), |b| b.name.to_string()),
                    ),
                ],
            );
            ui.add_space(30.0);
            grid(
                ui,
                "level",
                vec![
                    kv("input rms", format!("{:.1} dBFS", s.rms_dbfs)),
                    kv("input peak", format!("{:.0} %", s.peak * 100.0)),
                    kv("clipping", format!("{:.2} %", s.clip_percent)),
                    kv("dropped buffers", s.dropped.to_string()),
                ],
            );
            ui.add_space(30.0);
            let mut rows = vec![kv("CPU, whole app", format!("{:.0} %", self.cpu_percent))];
            for (stage, p) in decdab_engine::Stage::ALL.iter().zip(s.load.percent) {
                use decdab_engine::Stage;
                if (*stage == Stage::Record && s.recording.is_none())
                    || (matches!(stage, Stage::Ofdm | Stage::Fic) && !self.decode)
                    || (*stage == Stage::Audio && self.dab.playing.is_none())
                {
                    continue;
                }
                rows.push(kv(
                    &format!("  {}", stage.name()),
                    if *stage == Stage::Source && running {
                        format!("{p:.0} % (waiting)")
                    } else {
                        format!("{p:.1} %")
                    },
                ));
            }
            if let Some(r) = &s.recording {
                rows.push(kv("recording", format!("{:.0} MB", r.bytes as f64 / 1e6)));
            }
            grid(ui, "cpu", rows);
        });
        ui.add_space(4.0);
    }

    fn services_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        card::show(ui, &self.card());
        let ens = Arc::clone(&self.ensemble);
        let mut action = None;
        if let Some(sid) = self.dab.playing.or(self.want_play)
            && let Some(svc) = ens.services.get(&sid)
        {
            ui.add_space(8.0);
            let np = services::NowPlaying {
                service: svc,
                stats: self.dab.service.as_ref(),
                dls: self.dab.dls.as_ref(),
                slide: self.slide_tex.as_ref().map(|(_, t)| t),
                logo: self.logos.get(ui.ctx(), sid, 128),
                big_logo: self.logos.get(ui.ctx(), sid, 320),
                slide_name: self.dab.slide.as_ref().map(|s| s.name.as_str()),
                audio: self.dab.audio.as_ref(),
            };
            let (before, conceal_before) = (self.volume, self.mp2_conceal);
            action = services::now_playing(
                ui,
                &np,
                services::PlayingControls {
                    volume: &mut self.volume,
                    mp2_conceal: &mut self.mp2_conceal,
                },
            );
            if self.volume != before {
                prefs::set_volume(self.volume);
            }
            if self.mp2_conceal != conceal_before {
                prefs::set("mp2_conceal", if self.mp2_conceal { "1" } else { "0" });
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Services").strong());
            let n = ens.services.len();
            if n > 0 {
                let kbps: u32 = ens.subchannels.values().map(|s| s.bitrate()).sum();
                ui.label(RichText::new(format!("{n} · {kbps} kbit/s")).weak());
            }
        });
        ui.separator();
        if let Some(a) = services::list(ui, &ens, &mut self.selected, self.dab.playing, &mut self.logos) {
            action = Some(a);
        }
        match action {
            Some(services::Action::Play(sid)) => {
                self.want_play = Some(sid);
            }
            Some(services::Action::Stop) => {
                self.want_play = None;
            }
            None => {}
        }
    }
}

/// The SId a pattern names: hex SId, or part of the label.
fn find_service(e: &Ensemble, pat: &str) -> Option<u32> {
    if let Ok(sid) = u32::from_str_radix(pat.trim(), 16)
        && e.services.contains_key(&sid)
    {
        return Some(sid);
    }
    let p = pat.to_lowercase().replace(' ', "");
    e.services_by_name()
        .into_iter()
        .find(|s| s.name().to_lowercase().replace(' ', "").contains(&p))
        .map(|s| s.sid)
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.kind == SourceKind::HackRf {
            self.radio = (self.block, self.tune_hz);
        }
        let now = self.snapshot();
        if let Some(k) = self.keeper.as_mut() {
            k.flush(now);
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .first()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
        });
        if let Some(p) = dropped {
            self.open_file(p);
        }
        self.take_rows(ctx);
        self.sync_decoder();
        self.sync_play();
        self.sync_conceal();
        self.sync_scopes();
        self.sync_sounder();
        self.sync_tii();
        self.sync_sdr();
        self.sync_fps();
        self.optimise_tick();
        self.apply_dial(ctx);
        self.remember(ctx);
        self.cpu_percent = self.cpu.poll();
        self.handle_screenshot(ctx);
        if let Some(r) = &mut self.recorder
            && r.tick(ctx, self.started)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.session.is_some() || self.opts.test_run() {
            ctx.request_repaint_after(self.pace());
        }
        self.frame_cost[0] += t0.elapsed();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t0 = Instant::now();
        self.frames += 1;
        egui::Panel::top("controls").show(ui, |ui| self.control_bar(ui));
        egui::Panel::bottom("receiver").show(ui, |ui| self.status_panel(ui));
        if self.bottom == Bottom::Scopes {
            let header = (self.decode && self.dab.locked).then(|| {
                let mut bits = vec![
                    "DQPSK".to_string(),
                    format!("MER {:.1} dB", self.dab.mer_db),
                    format!("FIBs {:.0} % good", self.dab.fib_ok_ratio * 100.0),
                ];
                if let Some(s) = &self.dab.service {
                    bits.push(format!("audio frames {} good · {} bad", s.au_ok, s.au_bad));
                }
                bits.join(" · ")
            });
            egui::Panel::bottom("scopes")
                .resizable(false)
                .show(ui, |ui| {
                    ui.set_min_height(SCOPES_PANEL_H);
                    let playing = self
                        .dab
                        .playing
                        .and_then(|sid| self.ensemble.services.get(&sid))
                        .map(|s| s.name());
                    self.scope.show(ui, header, playing.as_deref());
                });
        }
        if self.bottom == Bottom::Channel {
            let status = (self.decode && self.dab.locked).then(|| {
                format!(
                    "decision-directed from the DQPSK, a row every 12 ms · MER {:.1} dB · carrier {:+.1} Hz",
                    self.dab.mer_db, self.dab.cfo_hz
                )
            });
            egui::Panel::bottom("channel")
                .resizable(false)
                .show(ui, |ui| {
                    ui.set_min_height(SCOPES_PANEL_H + 20.0);
                    self.chan_view.show(ui, status);
                });
        }
        if self.bottom == Bottom::Sfn {
            egui::Panel::bottom("sfn")
                .resizable(false)
                .show(ui, |ui| {
                    ui.set_min_height(SCOPES_PANEL_H);
                    self.sfn_view.show(ui, SCOPES_PANEL_H - 8.0);
                });
        }
        if self.bottom == Bottom::Guide {
            egui::Panel::bottom("guide")
                .resizable(false)
                .show(ui, |ui| {
                    ui.set_min_height(SCOPES_PANEL_H);
                    ui.set_max_height(SCOPES_PANEL_H);
                    let ens = Arc::clone(&self.ensemble);
                    self.guide
                        .show(ui, &ens, self.dab.playing.or(self.want_play), &mut self.logos);
                });
        }
        egui::Panel::right("services")
            .resizable(true)
            .default_size(400.0)
            .min_size(320.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("right")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.services_panel(ui));
            });

        let mut tune = None;
        let mux = if self.decode && self.dab.locked {
            mux_map::segments(&self.ensemble, self.dab.playing.or(self.want_play))
        } else {
            Vec::new()
        };
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                self.wf_rows =
                    band::waterfall_rows(ui.available_height(), ui.ctx().pixels_per_point())
                        .max(64);
                let tex = self.ring.update(ui.ctx(), &self.history);
                let inp = band::BandInput {
                    spectrum_db: &self.smooth,
                    center_hz: self.centre_hz,
                    span_hz: self.span_hz,
                    levels: self.history.levels(),
                    waterfall: tex,
                    vfo_hz: self.vfo_hz,
                    mux: &mux,
                };
                match band::show(ui, &inp) {
                    Some(band::Action::Tune(b)) => tune = Some(b),
                    Some(band::Action::Play(sid)) => {
                        self.want_play = Some(sid);
                        self.selected = Some(sid);
                    }
                    None => {}
                }
            });
        let ctx = ui.ctx().clone();
        if self.sdr_open {
            let live = self.session.as_ref().is_some_and(|s| s.live);
            let asks = sdr::show(
                &ctx,
                &mut self.sdr_open,
                sdr::SdrView {
                    settings: &mut self.sdr,
                    gains: &mut self.gains,
                    live,
                    front_end: self.stats.front_end,
                    optimiser: self.optimiser.as_ref(),
                    last_result: self.opt_result.as_deref(),
                },
            );
            if asks.gains_changed {
                self.change_gains();
            }
            if asks.optimise {
                self.start_optimiser();
            }
            if asks.cancel {
                self.cancel_optimiser();
            }
            if asks.reset {
                self.sdr = sdr::SdrSettings::default();
                self.gains = Gains::default();
                self.change_gains();
            }
        }
        if let Some(b) = tune {
            if self.session.is_none() && self.kind == SourceKind::HackRf {
                self.block = b;
                self.block_text = b.name.to_string();
                self.tune_hz = b.hz();
                self.vfo_hz = Some(b.hz());
            } else {
                self.tune_block(b);
            }
        }
        self.frame_cost[1] += t0.elapsed();
    }
}

/// "20261024-094512" (UTC) for file names (Howard Hinnant's civil-from-days).
fn utc_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_utc_stamp_is_well_formed() {
        let s = utc_stamp();
        assert_eq!(s.len(), 15, "{s}");
        assert_eq!(&s[8..9], "-");
    }
}
