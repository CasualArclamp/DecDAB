// made by CasualArclamp
//! Simple mode: DecDAB as a plain DAB radio. The expert window's panels go
//! (spectrum, waterfall, card, scopes) and two screens take the window, in
//! the expert window's own look (dark panels, amber):
//!
//! - **Now playing**: the stations down the left, grouped by ensemble; the
//!   slideshow, the station's name, DL Plus's title and artist and the
//!   dynamic label in the middle; the clock and the signal on the right.
//! - **Stations & scan**: a sweep of Band III (each block's in-band level
//!   over the gap beside it), then each block with a signal locked in turn
//!   until its services hold. The list it finds is kept in
//!   `%APPDATA%\DecDAB\stations.txt`.
//!
//! After a full scan on a live HackRF or Airspy, the gain optimiser runs on
//! the ensemble with the best margin (the HackRF's LNA/VGA search, the
//! Airspy's preset walk), and the radio keeps the gains it finds.
//!
//! Keys: ▲▼ or PgUp/PgDn the previous or next station, 1–9 a station by
//! number, M mute, Tab the other screen, Esc now playing; on the scan screen
//! Enter starts and stops the scan.
//!
//! Cost: nothing new runs per sample. While simple mode shows, the scopes,
//! the sounder and TII are off and the waterfall's texture is not updated;
//! the screens are painted shapes and text. The sweep turns the receiver
//! off and reads the spectrum rows the engine makes anyway.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use decdab_core::channel::{Block, block_at, parse_block, scan_blocks};
use decdab_fic::{Ensemble, Service, Transport, pty_name};
use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, Painter, Rect, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

use crate::{App, SourceKind, prefs};

const BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const PANEL: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1b);
const BAR: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
const LINE: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xe8);
const SOFT: Color32 = Color32::from_rgb(0xc4, 0xc4, 0xc4);
const DIM: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
const AMBER: Color32 = Color32::from_rgb(0xf0, 0xc0, 0x40);
const ORANGE: Color32 = Color32::from_rgb(0xff, 0x9a, 0x1f);
const FOCUS_BG: Color32 = Color32::from_rgb(0x3a, 0x2f, 0x12);
const FOCUS: Color32 = Color32::from_rgb(0xff, 0xd3, 0x6a);
const DABPLUS: Color32 = Color32::from_rgb(0xb8, 0xa3, 0xff);
const DAB: Color32 = Color32::from_rgb(0x8f, 0xd0, 0xff);
const GOOD: Color32 = Color32::from_rgb(0x9a, 0xd0, 0xa0);

/// A block whose in-band level is this far over the gap beside it is worth
/// locking to (the CLI's `scan` says "DAB?" from 6 dB).
const OVER_GAP_DB: f32 = 6.0;
/// How long the sweep listens to a block after the first row from it.
const DWELL: Duration = Duration::from_millis(450);
/// A block that sends no rows in this long is passed.
const SWEEP_GIVE_UP: Duration = Duration::from_secs(3);
/// An ensemble's services are taken once their count has held this long.
const SETTLE: Duration = Duration::from_millis(2500);
/// A block that gives no services in this long is passed.
const LOCK_GIVE_UP: Duration = Duration::from_secs(12);
/// The optimiser starts once the ensemble has been locked this long, and is
/// given up if there is no lock in the second.
const OPT_WAIT: Duration = Duration::from_secs(2);
const OPT_GIVE_UP: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Playing,
    Scan,
}

impl Screen {
    pub fn parse(s: &str) -> Option<Screen> {
        match s {
            "playing" => Some(Screen::Playing),
            "scan" => Some(Screen::Scan),
            _ => None,
        }
    }
}

/// A station a scan found.
#[derive(Clone, Debug)]
pub struct Station {
    pub sid: u32,
    pub name: String,
    /// DAB+ (else DAB, MPEG Layer II).
    pub plus: bool,
}

/// An ensemble a scan found: its block, label, margin when it was read and
/// its stations, by name.
#[derive(Clone, Debug)]
pub struct Mux {
    pub block: Block,
    pub name: String,
    pub margin_db: Option<f32>,
    pub stations: Vec<Station>,
}

enum Phase {
    /// Stepping over Band III: the block being listened to, when it was
    /// tuned, and the tuning count before it.
    Sweep {
        at: usize,
        tuned: Instant,
        tuning: u32,
        heard: Option<Instant>,
    },
    /// Locking to each block with a signal in turn.
    Lock {
        cands: Vec<usize>,
        at: usize,
        since: Instant,
        seen: usize,
        stable: Instant,
    },
    /// The gain optimiser on the strongest ensemble (its block index), then
    /// the station to play.
    Optimise {
        at: usize,
        pick: Option<(Block, u32)>,
        since: Instant,
        started: bool,
    },
}

struct Scan {
    phase: Phase,
    /// Where to go back to if the scan is stopped.
    back: (Block, Option<u32>),
    /// One block only, merged into the list (else the list is replaced).
    one: bool,
    found: Vec<Mux>,
}

pub struct Simple {
    pub on: bool,
    pub screen: Screen,
    scan: Option<Scan>,
    /// Every block's level over its gap from the last sweep, dB.
    levels: Vec<Option<f32>>,
    /// The blocks that were locked to and what each gave ("14 stations",
    /// "no lock"), by block index.
    states: Vec<(usize, String)>,
    /// The station list (the last scan's).
    pub found: Vec<Mux>,
    /// Something was put on since simple mode came on.
    started: bool,
    /// The radio was started for simple mode.
    autostarted: bool,
    /// The volume before mute.
    muted_from: Option<f32>,
    /// The local time offset the ensemble last sent, half hours.
    lto: Option<i8>,
    /// `DECDAB_SIMPLE_SCAN=1` (screenshots, tests) has started its scan.
    env_scan_done: bool,
}

impl Simple {
    pub fn new(on: bool, screen: Screen) -> Simple {
        Simple {
            on,
            screen,
            scan: None,
            levels: vec![None; scan_blocks().count()],
            states: Vec::new(),
            found: load_list(),
            started: false,
            autostarted: false,
            muted_from: None,
            lto: None,
            env_scan_done: false,
        }
    }

    /// The sweep has the receiver off (it only needs the spectrum).
    pub fn sweeping(&self) -> bool {
        matches!(
            self.scan.as_ref().map(|s| &s.phase),
            Some(Phase::Sweep { .. })
        )
    }
}

// ---- the station list file ----

fn list_path() -> Option<std::path::PathBuf> {
    Some(prefs::dir()?.join("stations.txt"))
}

/// The station list: a line a station,
/// "12B<TAB>ensemble<TAB>margin dB or -<TAB>SId hex<TAB>1 for DAB+<TAB>name".
fn load_list() -> Vec<Mux> {
    list_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map_or_else(Vec::new, |t| parse_list(&t))
}

fn parse_list(text: &str) -> Vec<Mux> {
    let mut out: Vec<Mux> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(6, '\t').collect();
        let [block, ens, margin, sid, plus, name] = f[..] else {
            continue;
        };
        let (Some(block), Ok(sid)) = (parse_block(block), u32::from_str_radix(sid, 16)) else {
            continue;
        };
        let st = Station {
            sid,
            name: name.to_string(),
            plus: plus == "1",
        };
        match out.iter_mut().find(|m| m.block == block) {
            Some(m) => m.stations.push(st),
            None => out.push(Mux {
                block,
                name: ens.to_string(),
                margin_db: margin.parse().ok(),
                stations: vec![st],
            }),
        }
    }
    out
}

fn list_text(found: &[Mux]) -> String {
    let mut s = String::new();
    for m in found {
        let margin = m.margin_db.map_or("-".to_string(), |d| format!("{d:.1}"));
        for st in &m.stations {
            s.push_str(&format!(
                "{}\t{}\t{margin}\t{:X}\t{}\t{}\n",
                m.block.name,
                m.name,
                st.sid,
                u8::from(st.plus),
                st.name
            ));
        }
    }
    s
}

fn save_list(found: &[Mux]) {
    let Some(p) = list_path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    let _ = std::fs::write(p, list_text(found));
}

/// A service that can be played: DAB+ or DAB audio.
fn audio_kind(s: &Service) -> Option<bool> {
    match s.primary().map(|c| c.transport) {
        Some(Transport::Audio { ascty, .. }) => Some(ascty == 63),
        _ => None,
    }
}

/// The ensemble's playable stations, those with a label, by name.
fn stations_of(e: &Ensemble) -> Vec<Station> {
    e.services_by_name()
        .into_iter()
        .filter(|s| s.label.is_some())
        .filter_map(|s| {
            Some(Station {
                sid: s.sid,
                name: s.name(),
                plus: audio_kind(s)?,
            })
        })
        .collect()
}

// ---- time ----

/// The local time ("12:43") and date ("SUN 1 NOV"), from the system's UTC
/// clock and the ensemble's local time offset.
fn clock(lto: Option<i8>) -> (String, String) {
    let utc = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let t = utc + lto.unwrap_or(0) as i64 * 1800;
    let (days, secs) = (t.div_euclid(86400), t.rem_euclid(86400));
    let (_, m, d) = civil(days);
    const WD: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
    const MON: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    (
        format!("{:02}:{:02}", secs / 3600, secs / 60 % 60),
        format!(
            "{} {d} {}{}",
            WD[(days + 4).rem_euclid(7) as usize],
            MON[(m - 1) as usize],
            if lto.is_none() { " UTC" } else { "" }
        ),
    )
}

/// (year, month, day) from days since 1970-01-01 (Howard Hinnant's
/// civil-from-days).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "14 stations · 1 ensemble".
fn count(found: &[Mux]) -> String {
    let n: usize = found.iter().map(|m| m.stations.len()).sum();
    let e = found.len();
    format!(
        "{n} station{} · {e} ensemble{}",
        if n == 1 { "" } else { "s" },
        if e == 1 { "" } else { "s" }
    )
}

/// A line of the scan screen's table of ensembles.
struct TableRow {
    block: Block,
    name: String,
    stations: String,
    margin: Option<f32>,
    state: String,
    colour: Color32,
    current: bool,
}

impl TableRow {
    fn of(m: &Mux, state: &str, colour: Color32) -> TableRow {
        TableRow {
            block: m.block,
            name: m.name.clone(),
            stations: m.stations.len().to_string(),
            margin: m.margin_db,
            state: state.into(),
            colour,
            current: false,
        }
    }

    /// A block with no ensemble (yet).
    fn bare(block: Block, state: &str, colour: Color32) -> TableRow {
        TableRow {
            block,
            name: "—".into(),
            stations: String::new(),
            margin: None,
            state: state.into(),
            colour,
            current: false,
        }
    }
}

/// What a click on a screen asks for, done once the drawing is over.
enum Ask {
    Play(Block, u32),
    Step(bool),
    Mute,
    Record,
    Scan,
    StopScan,
    RescanHere,
    Screen(Screen),
    Expert,
}

impl App {
    // ---- switching ----

    /// Simple mode on or off (remembered by a run that keeps its settings).
    pub(crate) fn simple_set(&mut self, on: bool) {
        self.simple.on = on;
        if on {
            self.simple.started = self.dab.playing.is_some() || self.want_play.is_some();
        } else {
            self.simple_stop_scan();
        }
        if self.keeper.is_some() {
            prefs::set("simple", if on { "1" } else { "0" });
        }
    }

    /// The block the receiver is on.
    fn here(&self) -> Option<Block> {
        self.vfo_hz.and_then(block_at)
    }

    /// Play station `sid` of the ensemble on `block`, tuning there first if
    /// it is another one's.
    fn simple_play(&mut self, block: Block, sid: u32) {
        self.simple.screen = Screen::Playing;
        self.simple.started = true;
        if self.keeper.is_some() {
            prefs::set_many(&[
                ("simple_sid", &format!("{sid:X}")),
                ("simple_block", block.name),
            ]);
        }
        if self.here() == Some(block) && self.ensemble.services.contains_key(&sid) {
            self.want_play = Some(sid);
            self.selected = Some(sid);
        } else {
            if self.here() != Some(block) {
                self.tune_block(block);
            }
            // Played once the ensemble lists it.
            self.opts.play = Some(format!("{sid:X}"));
        }
    }

    /// The stations in the order the list shows them: the scan's, else the
    /// ensemble's on the air.
    fn zap_list(&self) -> Vec<(Block, u32)> {
        if !self.simple.found.is_empty() {
            return self
                .simple
                .found
                .iter()
                .flat_map(|m| m.stations.iter().map(move |s| (m.block, s.sid)))
                .collect();
        }
        let Some(b) = self.here() else {
            return Vec::new();
        };
        stations_of(&self.ensemble)
            .into_iter()
            .map(|s| (b, s.sid))
            .collect()
    }

    fn zap(&mut self, up: bool) {
        let list = self.zap_list();
        if list.is_empty() {
            return;
        }
        let now = self.dab.playing.or(self.want_play);
        let at = list
            .iter()
            .position(|&(b, s)| Some(b) == self.here() && Some(s) == now);
        let n = list.len();
        let next = match at {
            Some(i) if up => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
            None => 0,
        };
        let (b, sid) = list[next];
        self.simple_play(b, sid);
    }

    fn simple_mute(&mut self) {
        match self.simple.muted_from.take() {
            Some(v) => self.volume = v,
            None => {
                self.simple.muted_from = Some(self.volume);
                self.volume = 0.0;
            }
        }
        decdab_audio::set_volume(self.volume);
    }

    // ---- every frame ----

    pub(crate) fn simple_tick(&mut self) {
        if !self.simple.on {
            return;
        }
        if let Some(l) = self.ensemble.lto_half_hours {
            self.simple.lto = Some(l);
        }
        // A radio: it starts with simple mode (a file starts as opened).
        if self.session.is_none() && !self.simple.autostarted && self.kind.is_radio() {
            self.simple.autostarted = true;
            self.start();
        }
        // DECDAB_SIMPLE_SCAN=1 (screenshots, tests): scan once the source
        // runs.
        if !self.simple.env_scan_done
            && std::env::var_os("DECDAB_SIMPLE_SCAN").is_some()
            && self.session.is_some()
            && self.started.elapsed() > Duration::from_secs(2)
        {
            self.simple.env_scan_done = true;
            self.simple_start_scan(false);
        }
        self.scan_tick();
        // Something on: the last station if this ensemble has it, else its
        // first.
        if !self.simple.started
            && self.simple.scan.is_none()
            && self.opts.play.is_none()
            && self.want_play.is_none()
            && let Some(b) = self.here()
        {
            let list = stations_of(&self.ensemble);
            let last = prefs::get("simple_sid").and_then(|s| u32::from_str_radix(&s, 16).ok());
            let pick = last
                .filter(|l| list.iter().any(|s| s.sid == *l))
                .or(list.first().map(|s| s.sid));
            if let Some(sid) = pick {
                self.simple_play(b, sid);
            }
        }
    }

    // ---- the scan ----

    /// Scan Band III (`one`: lock to this block only and merge what it has).
    pub(crate) fn simple_start_scan(&mut self, one: bool) {
        if self.simple.scan.is_some() {
            return;
        }
        if self.session.is_none() {
            self.start();
            if self.session.is_none() {
                return;
            }
        }
        let here = self.here().unwrap_or(self.block);
        let blocks: Vec<Block> = scan_blocks().collect();
        let back = (here, self.dab.playing.or(self.want_play));
        self.want_play = None;
        self.opts.play = None;
        self.simple.screen = Screen::Scan;
        let live = self.session.as_ref().is_some_and(|s| s.live);
        let idx = blocks.iter().position(|b| *b == here);
        // A file cannot be retuned: its one ensemble is read.
        let phase = if one || !live {
            let Some(i) = idx else {
                self.note = "this capture is not on a Band III block".into();
                return;
            };
            self.simple.states.retain(|(b, _)| *b != i);
            Phase::Lock {
                cands: vec![i],
                at: 0,
                since: Instant::now(),
                seen: 0,
                stable: Instant::now(),
            }
        } else {
            self.simple.levels = vec![None; blocks.len()];
            self.simple.states.clear();
            let tuning = self.tuning;
            self.set_frequency(blocks[0].hz());
            Phase::Sweep {
                at: 0,
                tuned: Instant::now(),
                tuning,
                heard: None,
            }
        };
        self.simple.scan = Some(Scan {
            phase,
            back,
            one: one || !live,
            found: Vec::new(),
        });
    }

    fn simple_stop_scan(&mut self) {
        let Some(s) = self.simple.scan.take() else {
            return;
        };
        self.note = "scan stopped".into();
        self.cancel_optimiser();
        let (b, sid) = s.back;
        match sid {
            Some(sid) => self.simple_play(b, sid),
            None => self.tune_block(b),
        }
    }

    /// The level of the block at the centre of the spectrum over the gap
    /// beside it (from the CLI's scan): the carriers away from DC against
    /// the space between this block's edge (768 kHz) and the next's.
    fn block_margin(&self) -> Option<f32> {
        let n = self.smooth.len();
        if n == 0 || self.span_hz <= 0.0 {
            return None;
        }
        let bin = self.span_hz / n as f64;
        let band = |lo: f64, hi: f64| {
            let (mut s, mut k) = (0f64, 0usize);
            for (i, &d) in self.smooth.iter().enumerate() {
                let f = ((i as f64 + 0.5) * bin - self.span_hz / 2.0).abs();
                if f >= lo && f < hi {
                    s += 10f64.powf(d as f64 / 10.0);
                    k += 1;
                }
            }
            (k > 0).then(|| (10.0 * (s / k as f64).log10()) as f32)
        };
        Some(band(20e3, 700e3)? - band(800e3, 910e3)?)
    }

    fn scan_tick(&mut self) {
        let blocks: Vec<Block> = scan_blocks().collect();
        let margin = self.block_margin();
        let tuning = self.tuning;
        let card = self.card();
        let Some(scan) = &mut self.simple.scan else {
            return;
        };
        if let Phase::Optimise {
            at,
            pick,
            since,
            started,
        } = &mut scan.phase
        {
            let (at, pick) = (*at, *pick);
            if !*started {
                let locked_long = self.dab.locked && since.elapsed() >= OPT_WAIT;
                if locked_long {
                    *started = true;
                    self.start_optimiser();
                    self.note = format!("optimising the gain on {}", blocks[at].name);
                    return;
                }
                if since.elapsed() < OPT_GIVE_UP {
                    return;
                }
                self.note = format!("no lock on {} to optimise the gain", blocks[at].name);
            } else if self.optimising().is_some() {
                return;
            } else if let Some(r) = &self.opt_result {
                self.note = format!("gain on {}: {r}", blocks[at].name);
            }
            if self.keeper.is_none() {
                eprintln!("simple scan: {}", self.note);
            }
            self.simple.scan = None;
            match pick {
                Some((b, sid)) => self.simple_play(b, sid),
                None => self.simple.started = false,
            }
            return;
        }
        match &mut scan.phase {
            // Handled above.
            Phase::Optimise { .. } => {}
            Phase::Sweep {
                at,
                tuned,
                tuning: before,
                heard,
            } => {
                if tuning != *before && heard.is_none() && !self.smooth.is_empty() {
                    *heard = Some(Instant::now());
                }
                let done = match heard {
                    Some(h) => h.elapsed() >= DWELL,
                    None => tuned.elapsed() >= SWEEP_GIVE_UP,
                };
                if !done {
                    return;
                }
                self.simple.levels[*at] = heard.and(margin);
                *at += 1;
                if let Some(b) = blocks.get(*at) {
                    (*tuned, *before, *heard) = (Instant::now(), tuning, None);
                    let hz = b.hz();
                    self.set_frequency(hz);
                    return;
                }
                let cands: Vec<usize> = (0..blocks.len())
                    .filter(|&i| self.simple.levels[i].is_some_and(|l| l > OVER_GAP_DB))
                    .collect();
                // A test run (screenshots) logs the sweep, for checking it.
                if self.keeper.is_none() {
                    let lv: Vec<String> = blocks
                        .iter()
                        .zip(&self.simple.levels)
                        .map(|(b, l)| {
                            format!("{} {}", b.name, l.map_or("-".into(), |l| format!("{l:.1}")))
                        })
                        .collect();
                    eprintln!("simple scan: sweep {}", lv.join(", "));
                }
                let Some(&first) = cands.first() else {
                    self.finish_scan();
                    return;
                };
                scan.phase = Phase::Lock {
                    cands,
                    at: 0,
                    since: Instant::now(),
                    seen: 0,
                    stable: Instant::now(),
                };
                self.tune_block(blocks[first]);
            }
            Phase::Lock {
                cands,
                at,
                since,
                seen,
                stable,
            } => {
                let list = stations_of(&self.ensemble);
                if list.len() != *seen {
                    *seen = list.len();
                    *stable = Instant::now();
                }
                let settled = !list.is_empty()
                    && stable.elapsed() >= SETTLE
                    && since.elapsed() >= Duration::from_secs(3);
                if !settled && since.elapsed() < LOCK_GIVE_UP {
                    return;
                }
                let i = cands[*at];
                if self.keeper.is_none() {
                    eprintln!(
                        "simple scan: {} {} after {:.1} s: {} stations, MER {:.1} dB",
                        blocks[i].name,
                        self.ensemble.name(),
                        since.elapsed().as_secs_f32(),
                        list.len(),
                        self.dab.mer_db
                    );
                }
                if list.is_empty() {
                    self.simple.states.push((i, "no lock".into()));
                } else {
                    let n = list.len();
                    self.simple
                        .states
                        .push((i, format!("{n} station{}", if n == 1 { "" } else { "s" })));
                    scan.found.push(Mux {
                        block: blocks[i],
                        name: self.ensemble.name(),
                        margin_db: card.cn_db.zip(card.qef_db).map(|(c, q)| c - q),
                        stations: list,
                    });
                }
                *at += 1;
                if let Some(&next) = cands.get(*at) {
                    (*since, *seen, *stable) = (Instant::now(), 0, Instant::now());
                    self.tune_block(blocks[next]);
                } else {
                    self.finish_scan();
                }
            }
        }
    }

    fn finish_scan(&mut self) {
        let Some(s) = self.simple.scan.take() else {
            return;
        };
        if s.one {
            for m in s.found {
                match self.simple.found.iter_mut().find(|f| f.block == m.block) {
                    Some(f) => *f = m,
                    None => self.simple.found.push(m),
                }
            }
            self.simple.found.sort_by_key(|m| m.block.khz);
        } else if !s.found.is_empty() {
            self.simple.found = s.found;
        }
        self.note = format!("scan done: {}", count(&self.simple.found));
        if self.keeper.is_some() {
            save_list(&self.simple.found);
        }
        // Back to what was playing if it is still listed, else the first.
        let (b, sid) = s.back;
        let listed = |b: Block, sid: u32| {
            self.simple
                .found
                .iter()
                .any(|m| m.block == b && m.stations.iter().any(|s| s.sid == sid))
        };
        let pick = sid
            .filter(|&sid| listed(b, sid))
            .map(|sid| (b, sid))
            .or_else(|| {
                let m = self.simple.found.first()?;
                Some((m.block, m.stations.first()?.sid))
            });
        // A full scan on a live HackRF or Airspy: the gain optimised on the
        // strongest ensemble before playing.
        let live = self.session.as_ref().is_some_and(|s| s.live);
        let strongest = self
            .simple
            .found
            .iter()
            .filter_map(|m| Some((m.block, m.margin_db?)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(b, _)| b);
        let blocks: Vec<Block> = scan_blocks().collect();
        let at = strongest.and_then(|sb| blocks.iter().position(|b| *b == sb));
        if let Some(at) = at
            && !s.one
            && live
            && self.kind != SourceKind::RtlSdr
        {
            self.tune_block(blocks[at]);
            self.note = format!(
                "{} · waiting for {} to optimise the gain",
                self.note, blocks[at].name
            );
            self.simple.scan = Some(Scan {
                phase: Phase::Optimise {
                    at,
                    pick,
                    since: Instant::now(),
                    started: false,
                },
                back: s.back,
                one: false,
                found: Vec::new(),
            });
            return;
        }
        match pick {
            Some((b, sid)) => self.simple_play(b, sid),
            None => self.tune_block(b),
        }
    }

    // ---- keys ----

    fn simple_keys(&mut self, ctx: &egui::Context) -> Option<Ask> {
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }
        let press = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if press(Key::Tab) {
            return Some(Ask::Screen(match self.simple.screen {
                Screen::Playing => Screen::Scan,
                Screen::Scan => Screen::Playing,
            }));
        }
        if press(Key::Escape) {
            return Some(Ask::Screen(Screen::Playing));
        }
        if press(Key::ArrowUp) || press(Key::PageUp) {
            return Some(Ask::Step(true));
        }
        if press(Key::ArrowDown) || press(Key::PageDown) {
            return Some(Ask::Step(false));
        }
        if press(Key::M) {
            return Some(Ask::Mute);
        }
        if self.simple.screen == Screen::Scan && press(Key::Enter) {
            return Some(if self.simple.scan.is_some() {
                Ask::StopScan
            } else {
                Ask::Scan
            });
        }
        const DIGITS: [Key; 9] = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (i, k) in DIGITS.into_iter().enumerate() {
            if press(k)
                && let Some(&(b, sid)) = self.zap_list().get(i)
            {
                return Some(Ask::Play(b, sid));
            }
        }
        None
    }

    fn simple_do(&mut self, ask: Ask) {
        match ask {
            Ask::Play(b, sid) => self.simple_play(b, sid),
            Ask::Step(up) => self.zap(up),
            Ask::Mute => self.simple_mute(),
            Ask::Record => self.toggle_mp3_record(),
            Ask::Scan => self.simple_start_scan(false),
            Ask::StopScan => self.simple_stop_scan(),
            Ask::RescanHere => self.simple_start_scan(true),
            Ask::Screen(s) => self.simple.screen = s,
            Ask::Expert => self.simple_set(false),
        }
    }

    // ---- drawing ----

    /// The whole window in simple mode: the strip, then the screen.
    pub(crate) fn simple_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut ask = self.simple_keys(&ctx);
        egui::Panel::top("simple_bar")
            .frame(
                egui::Frame::NONE
                    .fill(BAR)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ui, |ui| {
                if let Some(a) = self.simple_bar(ui) {
                    ask = Some(a);
                }
            });
        egui::Panel::bottom("simple_keys")
            .frame(
                egui::Frame::NONE
                    .fill(BAR)
                    .inner_margin(egui::Margin::symmetric(16, 8)),
            )
            .show(ui, |ui| self.key_hints(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG))
            .show(ui, |ui| {
                let a = match self.simple.screen {
                    Screen::Playing => self.draw_playing(ui),
                    Screen::Scan => self.draw_scan(ui),
                };
                if let Some(a) = a {
                    ask = Some(a);
                }
            });
        if let Some(a) = ask {
            self.simple_do(a);
        }
    }

    /// The strip along the top: the frequency, the screens, and back to the
    /// expert window.
    fn simple_bar(&mut self, ui: &mut egui::Ui) -> Option<Ask> {
        let mut ask = None;
        ui.horizontal(|ui| {
            let hz = self.vfo_hz.unwrap_or(self.tune_hz);
            ui.label(
                egui::RichText::new(format!("{:.3} MHz", hz / 1e6))
                    .size(16.0)
                    .color(Color32::from_gray(0xdd)),
            );
            ui.label(
                egui::RichText::new(self.here().map_or("", |b| b.name))
                    .size(12.0)
                    .color(DIM),
            );
            ui.separator();
            for (s, name) in [
                (Screen::Playing, "Now playing"),
                (Screen::Scan, "Stations & scan"),
            ] {
                if ui.selectable_label(self.simple.screen == s, name).clicked() {
                    ask = Some(Ask::Screen(s));
                }
            }
            if !self.note.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(&self.note).color(AMBER));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.selectable_label(false, "expert").clicked() {
                    ask = Some(Ask::Expert);
                }
                let _ = ui.selectable_label(true, "simple");
            });
        });
        ask
    }

    fn key_hints(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let hint = |ui: &mut egui::Ui, k: &str, what: &str| {
                ui.label(egui::RichText::new(k).color(AMBER).strong());
                ui.label(egui::RichText::new(what).color(Color32::from_gray(0xaa)));
                ui.add_space(18.0);
            };
            match self.simple.screen {
                Screen::Playing => {
                    hint(ui, "▲▼", "station");
                    hint(ui, "1-9", "station by number");
                    hint(ui, "M", "mute");
                    hint(ui, "Tab", "stations & scan");
                }
                Screen::Scan => {
                    hint(ui, "Enter", "start / stop");
                    hint(ui, "▲▼", "station");
                    hint(ui, "Esc", "now playing");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = if !self.simple.found.is_empty() {
                    count(&self.simple.found)
                } else {
                    "no station list yet: scan on the Stations & scan screen".into()
                };
                ui.label(egui::RichText::new(text).color(Color32::from_gray(0x88)));
            });
        });
    }

    /// The groups the station column shows: the scan's list, else the
    /// ensemble on the air.
    fn groups(&self) -> Vec<Mux> {
        if !self.simple.found.is_empty() {
            return self.simple.found.clone();
        }
        match self.here() {
            Some(b) if !self.ensemble.services.is_empty() => vec![Mux {
                block: b,
                name: self.ensemble.name(),
                margin_db: None,
                stations: stations_of(&self.ensemble),
            }],
            _ => Vec::new(),
        }
    }

    fn draw_playing(&mut self, ui: &mut egui::Ui) -> Option<Ask> {
        let rect = ui.max_rect();
        let mut ask = None;
        let left = Rect::from_min_size(rect.min, vec2(290.0, rect.height()));
        let right = Rect::from_min_max(pos2(rect.right() - 240.0, rect.top()), rect.max);
        let mid = Rect::from_min_max(
            pos2(left.right(), rect.top()),
            pos2(right.left(), rect.bottom()),
        );
        {
            let p = ui.painter();
            p.rect_filled(left, 0.0, PANEL);
            p.vline(left.right(), left.y_range(), Stroke::new(1.0, LINE));
            p.vline(right.left(), right.y_range(), Stroke::new(1.0, LINE));
        }
        if let Some(a) = self.station_column(ui, left) {
            ask = Some(a);
        }
        if let Some(a) = self.now_playing(ui, mid.shrink2(vec2(28.0, 24.0))) {
            ask = Some(a);
        }
        self.signal_column(ui.painter(), right.shrink2(vec2(22.0, 24.0)));
        ask
    }

    /// The stations down the left, grouped by ensemble; a click plays one.
    fn station_column(&mut self, ui: &mut egui::Ui, rect: Rect) -> Option<Ask> {
        let groups = self.groups();
        let mut ask = None;
        let playing = self.dab.playing.or(self.want_play);
        let here = self.here();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        if groups.is_empty() {
            child.painter().text(
                rect.left_top() + vec2(14.0, 20.0),
                Align2::LEFT_TOP,
                "no stations yet",
                FontId::proportional(15.0),
                DIM,
            );
            return None;
        }
        egui::ScrollArea::vertical()
            .id_salt("simple_stations")
            .auto_shrink([false, false])
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let w = rect.width();
                let mut n = 0;
                for g in &groups {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, 0.0, BAR);
                    p.hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, LINE));
                    p.text(
                        pos2(r.left() + 14.0, r.center().y),
                        Align2::LEFT_CENTER,
                        format!("{} · {}", g.block.name, g.name.to_uppercase()),
                        FontId::monospace(11.0),
                        DIM,
                    );
                    for s in &g.stations {
                        n += 1;
                        let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
                        let on = here == Some(g.block) && playing == Some(s.sid);
                        let p = ui.painter();
                        if on {
                            p.rect_filled(r, 0.0, FOCUS_BG);
                            p.rect_filled(
                                Rect::from_min_size(r.min, vec2(3.0, r.height())),
                                0.0,
                                AMBER,
                            );
                        } else if resp.hovered() {
                            p.rect_filled(r, 0.0, Color32::from_gray(0x24));
                        }
                        p.hline(
                            r.x_range(),
                            r.bottom() - 0.5,
                            Stroke::new(1.0, Color32::from_gray(0x24)),
                        );
                        p.text(
                            pos2(r.left() + 14.0, r.center().y),
                            Align2::LEFT_CENTER,
                            n.to_string(),
                            FontId::monospace(12.0),
                            DIM,
                        );
                        p.text(
                            pos2(r.left() + 44.0, r.center().y),
                            Align2::LEFT_CENTER,
                            &s.name,
                            FontId::proportional(16.0),
                            if on { FOCUS } else { Color32::from_gray(0xdd) },
                        );
                        let (tag, colour) = if s.plus {
                            ("DAB+", DABPLUS)
                        } else {
                            ("DAB", DAB)
                        };
                        let g2 = p.layout_no_wrap(tag.into(), FontId::monospace(10.0), BG);
                        let tr = Rect::from_min_size(
                            pos2(r.right() - 14.0 - g2.size().x - 10.0, r.center().y - 8.0),
                            vec2(g2.size().x + 10.0, 16.0),
                        );
                        p.rect_filled(tr, 3.0, colour);
                        p.galley(tr.center() - g2.size() / 2.0, g2, BG);
                        if resp.clicked() {
                            ask = Some(Ask::Play(g.block, s.sid));
                        }
                    }
                }
            });
        ask
    }

    /// The middle: the slideshow, the station, DL Plus and the label, and
    /// the controls under them.
    fn now_playing(&mut self, ui: &mut egui::Ui, rect: Rect) -> Option<Ask> {
        let mut ask = None;
        let sid = self.dab.playing.or(self.want_play);
        let ens = std::sync::Arc::clone(&self.ensemble);
        let svc = sid.and_then(|s| ens.services.get(&s));
        let p = ui.painter().clone();

        // The slideshow (or the station's big logo, or an empty frame).
        let slide = Rect::from_min_size(rect.min, vec2(400.0, 300.0));
        p.rect_filled(slide, 0.0, Color32::from_gray(0x0c));
        p.rect_stroke(slide, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let big = sid
            .and_then(|s| self.logos.get(ui.ctx(), s, 320))
            .filter(|t| t.size()[0] >= 120);
        match self.slide_tex.as_ref().map(|(_, t)| t.clone()).or(big) {
            Some(t) => {
                let [tw, th] = t.size();
                let s = (slide.width() / tw as f32).min(slide.height() / th as f32);
                let r = Rect::from_center_size(slide.center(), vec2(tw as f32, th as f32) * s);
                p.image(
                    t.id(),
                    r,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                p.text(
                    slide.center(),
                    Align2::CENTER_CENTER,
                    if svc.is_some() { "no slideshow" } else { "" },
                    FontId::proportional(14.0),
                    Color32::from_gray(0x66),
                );
            }
        }

        // The text beside it.
        let tx = slide.right() + 26.0;
        let tw = (rect.right() - tx).max(120.0);
        let mut y = rect.top();
        let line =
            |p: &Painter, y: &mut f32, text: String, font: FontId, colour: Color32, gap: f32| {
                let g = p.layout(text, font, colour, tw);
                let h = g.size().y;
                p.galley(pos2(tx, *y), g, colour);
                *y += h + gap;
            };
        match svc {
            Some(s) => {
                let mut head = vec![ens.name().to_uppercase(), format!("SID {:04X}", s.sid)];
                if let Some(t) = s.pty {
                    head.push(pty_name(t).to_uppercase());
                }
                line(
                    &p,
                    &mut y,
                    head.join(" · "),
                    FontId::monospace(13.0),
                    AMBER,
                    6.0,
                );
                line(&p, &mut y, s.name(), FontId::proportional(38.0), TEXT, 14.0);
                if let Some(l) = &self.dab.dls {
                    let tag = |k: u8| l.tags.iter().find(|(t, _)| *t == k).map(|(_, v)| v.clone());
                    if let Some(title) = tag(1) {
                        line(
                            &p,
                            &mut y,
                            "DL PLUS".into(),
                            FontId::monospace(12.0),
                            DIM,
                            2.0,
                        );
                        line(&p, &mut y, title, FontId::proportional(22.0), TEXT, 2.0);
                        if let Some(artist) = tag(4).or_else(|| tag(9)) {
                            line(&p, &mut y, artist, FontId::proportional(17.0), SOFT, 12.0);
                        } else {
                            y += 10.0;
                        }
                    }
                    line(&p, &mut y, "DLS".into(), FontId::monospace(12.0), DIM, 2.0);
                    line(
                        &p,
                        &mut y,
                        l.text.clone(),
                        FontId::proportional(16.0),
                        SOFT,
                        0.0,
                    );
                } else if self.dab.playing.is_some() {
                    line(
                        &p,
                        &mut y,
                        "no text from the station yet".into(),
                        FontId::proportional(15.0),
                        DIM,
                        0.0,
                    );
                } else {
                    line(
                        &p,
                        &mut y,
                        "tuning…".into(),
                        FontId::proportional(15.0),
                        DIM,
                        0.0,
                    );
                }
            }
            None => {
                let what = if self.simple.scan.is_some() {
                    "Scanning…"
                } else if self.session.is_none() {
                    "The radio is off"
                } else if self.dab.locked {
                    "Pick a station"
                } else {
                    "Looking for an ensemble…"
                };
                line(
                    &p,
                    &mut y,
                    what.into(),
                    FontId::proportional(30.0),
                    TEXT,
                    8.0,
                );
                if self.simple.found.is_empty() {
                    line(
                        &p,
                        &mut y,
                        "Scan Band III to find your stations (Tab).".into(),
                        FontId::proportional(16.0),
                        SOFT,
                        0.0,
                    );
                }
            }
        }

        // The controls.
        let cy = slide.bottom().max(y) + 24.0;
        p.hline(rect.x_range(), cy - 12.0, Stroke::new(1.0, LINE));
        let row = Rect::from_min_max(pos2(rect.left(), cy), pos2(rect.right(), cy + 34.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().button_padding = vec2(14.0, 7.0);
        if ui.button("◀ prev").clicked() {
            ask = Some(Ask::Step(true));
        }
        if ui.button("next ▶").clicked() {
            ask = Some(Ask::Step(false));
        }
        let muted = self.simple.muted_from.is_some();
        if ui.selectable_label(muted, "mute").clicked() {
            ask = Some(Ask::Mute);
        }
        ui.add_space(12.0);
        ui.label(egui::RichText::new("vol").color(DIM));
        let before = self.volume;
        ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false));
        if self.volume != before {
            self.simple.muted_from = None;
            decdab_audio::set_volume(self.volume);
            prefs::set_volume(self.volume);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let rec = self.stats.mp3_recording.is_some();
            let text = egui::RichText::new(if rec {
                "■ stop recording"
            } else {
                "● record"
            })
            .color(Color32::from_rgb(0xff, 0x8a, 0x80));
            if ui
                .add_enabled(self.dab.playing.is_some() || rec, egui::Button::new(text))
                .clicked()
            {
                ask = Some(Ask::Record);
            }
        });
        ask
    }

    /// The right: the clock, the margin over what the code needs, and how
    /// the station is coded and received.
    fn signal_column(&self, p: &Painter, rect: Rect) {
        let (time, date) = clock(self.simple.lto.or(self.ensemble.lto_half_hours));
        p.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            time,
            FontId::monospace(30.0),
            Color32::from_gray(0xdd),
        );
        p.text(
            rect.left_top() + vec2(0.0, 40.0),
            Align2::LEFT_TOP,
            date,
            FontId::monospace(12.0),
            DIM,
        );

        let c = self.card();
        let mut lines = Vec::new();
        let margin = c.cn_db.zip(c.qef_db).map(|(cn, q)| cn - q);
        if let Some(mer) = c.cn_db {
            lines.push(format!("MER {mer:.1} dB"));
        }
        if let Some(s) = &self.dab.service
            && !s.codec.is_empty()
        {
            lines.push(s.codec.clone());
        }
        if let Some(sub) = &self.dab.subchannel {
            lines.push(format!("{} · {} CU", sub.protection_name(), sub.size));
            lines.push(format!("{} kbit/s", sub.bitrate()));
        }
        if let Some(s) = &self.dab.service {
            lines.push(format!("audio errors {}", s.au_bad));
        }
        if !c.active {
            lines.push("radio off".into());
        } else if !self.dab.locked {
            lines.push("no lock".into());
        }
        let lh = 19.0;
        let mut y = rect.bottom() - lines.len() as f32 * lh;
        for l in &lines {
            p.text(
                pos2(rect.left(), y),
                Align2::LEFT_TOP,
                l,
                FontId::monospace(12.0),
                DIM,
            );
            y += lh;
        }
        let by = rect.bottom() - lines.len() as f32 * lh - 22.0;
        let bar = Rect::from_min_size(pos2(rect.left(), by), vec2(rect.width(), 6.0));
        p.rect_filled(bar, 0.0, LINE);
        if let Some(m) = margin {
            // 0 dB at the left (the code just copes), 8 dB over it full.
            let f = (m / 8.0).clamp(0.0, 1.0);
            let colour = if m < 1.0 { ORANGE } else { AMBER };
            p.rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * f, 6.0)),
                0.0,
                colour,
            );
        }
        p.text(
            pos2(rect.left(), by - 8.0),
            Align2::LEFT_BOTTOM,
            "margin",
            FontId::monospace(12.0),
            Color32::from_gray(0xaa),
        );
        p.text(
            pos2(rect.right(), by - 8.0),
            Align2::RIGHT_BOTTOM,
            margin.map_or("—".into(), |m| format!("{m:+.1} dB")),
            FontId::monospace(12.0),
            AMBER,
        );
    }

    fn draw_scan(&mut self, ui: &mut egui::Ui) -> Option<Ask> {
        let rect = ui.max_rect().shrink2(vec2(22.0, 18.0));
        let p = ui.painter().clone();
        let mut ask = None;
        let blocks: Vec<Block> = scan_blocks().collect();
        let running = self.simple.scan.is_some();

        // The heading and what the scan is doing.
        p.text(
            rect.left_top(),
            Align2::LEFT_TOP,
            "Channel scan",
            FontId::proportional(22.0),
            TEXT,
        );
        p.text(
            rect.left_top() + vec2(170.0, 7.0),
            Align2::LEFT_TOP,
            "1 sweep 174–240 MHz · 2 lock each block over its gap until the FIC settles",
            FontId::monospace(12.0),
            DIM,
        );
        let (status, current) = match self.simple.scan.as_ref().map(|s| &s.phase) {
            Some(Phase::Sweep { at, .. }) => (
                format!(
                    "sweeping {} · {} of {}",
                    blocks[*at].name,
                    at + 1,
                    blocks.len()
                ),
                Some(*at),
            ),
            Some(Phase::Lock { cands, at, .. }) => (
                format!(
                    "locking {} · {} of {}",
                    blocks[cands[*at]].name,
                    at + 1,
                    cands.len()
                ),
                Some(cands[*at]),
            ),
            Some(Phase::Optimise { at, started, .. }) => (
                match (started, self.optimising()) {
                    (true, Some((n, best))) => format!(
                        "optimising the gain on {} · {n} settings tried{}",
                        blocks[*at].name,
                        best.map_or(String::new(), |m| format!(" · best MER {m:.1} dB"))
                    ),
                    _ => format!("waiting for {} to lock", blocks[*at].name),
                },
                Some(*at),
            ),
            None if self.simple.found.is_empty() => {
                ("press Enter to find your stations".into(), None)
            }
            None => ("your stations: Enter scans again".into(), None),
        };
        p.text(
            rect.right_top() + vec2(0.0, 6.0),
            Align2::RIGHT_TOP,
            status,
            FontId::monospace(13.0),
            AMBER,
        );

        // 1: the sweep, a bar a block: its level over the gap beside it.
        let chart =
            Rect::from_min_size(rect.left_top() + vec2(0.0, 44.0), vec2(rect.width(), 150.0));
        let n = blocks.len() as f32;
        let gap = 4.0;
        let bw = (chart.width() - gap * (n - 1.0)) / n;
        let scale = |db: f32| (db.clamp(0.0, 30.0) / 30.0) * (chart.height() - 16.0);
        let found_at = |b: Block| self.simple.found.iter().any(|m| m.block == b);
        for (i, b) in blocks.iter().enumerate() {
            let x = chart.left() + i as f32 * (bw + gap);
            let lvl = self.simple.levels[i];
            let h = lvl.map_or(3.0, |l| scale(l).max(3.0));
            let over = lvl.is_some_and(|l| l > OVER_GAP_DB);
            let colour = if current == Some(i) {
                ORANGE
            } else if over || (lvl.is_none() && found_at(*b)) {
                AMBER
            } else if lvl.is_some() {
                Color32::from_rgb(0x7a, 0x6a, 0x3a)
            } else {
                Color32::from_gray(0x3a)
            };
            let h = if lvl.is_none() && found_at(*b) {
                scale(12.0)
            } else {
                h
            };
            let r = Rect::from_min_max(pos2(x, chart.bottom() - h), pos2(x + bw, chart.bottom()));
            p.rect_filled(r, 0.0, colour);
            if over || current == Some(i) {
                p.text(
                    pos2(r.center().x, r.top() - 3.0),
                    Align2::CENTER_BOTTOM,
                    b.name,
                    FontId::monospace(10.0),
                    Color32::from_gray(0xdd),
                );
            }
            p.text(
                pos2(r.center().x, chart.bottom() + 4.0),
                Align2::CENTER_TOP,
                b.name,
                FontId::monospace(9.0),
                Color32::from_gray(0x88),
            );
        }
        p.hline(
            chart.x_range(),
            chart.bottom(),
            Stroke::new(1.0, Color32::from_gray(0x3a)),
        );
        let ty = chart.bottom() - scale(OVER_GAP_DB);
        let mut x = chart.left();
        while x < chart.right() {
            p.hline(
                x..=(x + 6.0).min(chart.right()),
                ty,
                Stroke::new(1.0, Color32::from_rgb(0x6a, 0x5a, 0x2a)),
            );
            x += 10.0;
        }
        p.text(
            pos2(chart.right(), ty - 3.0),
            Align2::RIGHT_BOTTOM,
            format!("gap + {OVER_GAP_DB:.0} dB"),
            FontId::monospace(10.0),
            Color32::from_rgb(0xa0, 0x8a, 0x4a),
        );

        // 2: the ensembles: the scan's blocks while it runs, else the list.
        let mut rows: Vec<TableRow> = Vec::new();
        match &self.simple.scan {
            Some(Scan {
                phase: Phase::Lock { cands, at, .. },
                found,
                ..
            }) => {
                for (k, &i) in cands.iter().enumerate() {
                    let b = blocks[i];
                    if let Some(m) = found.iter().find(|m| m.block == b) {
                        rows.push(TableRow::of(m, "done", GOOD));
                    } else if k == *at {
                        let mut r = TableRow::bare(b, "reading FIC", FOCUS);
                        let name = self.ensemble.name();
                        if self.dab.locked && !name.is_empty() {
                            r.name = name;
                        }
                        let n = stations_of(&self.ensemble).len();
                        r.stations = if n > 0 { n.to_string() } else { "…".into() };
                        r.current = true;
                        rows.push(r);
                    } else {
                        let state = if k < *at { "no lock" } else { "waiting" };
                        rows.push(TableRow::bare(b, state, DIM));
                    }
                }
            }
            Some(Scan {
                phase: Phase::Optimise { at, .. },
                ..
            }) => {
                for m in &self.simple.found {
                    let mut r = TableRow::of(m, "in the list", GOOD);
                    if m.block == blocks[*at] {
                        r.state = "optimising the gain".into();
                        r.colour = FOCUS;
                        r.current = true;
                    }
                    rows.push(r);
                }
            }
            Some(_) => {}
            None => {
                for m in &self.simple.found {
                    rows.push(TableRow::of(m, "in the list", GOOD));
                }
                for (i, st) in &self.simple.states {
                    if !self.simple.found.iter().any(|m| m.block == blocks[*i]) {
                        rows.push(TableRow::bare(blocks[*i], st, DIM));
                    }
                }
            }
        }
        let table = Rect::from_min_max(
            pos2(rect.left(), chart.bottom() + 30.0),
            pos2(rect.right(), rect.bottom() - 50.0),
        );
        p.rect_filled(table, 0.0, PANEL);
        p.rect_stroke(table, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let cols = [16.0, 96.0, 216.0, 520.0, 640.0, 800.0];
        let head = Rect::from_min_size(table.min, vec2(table.width(), 30.0));
        p.rect_filled(head, 0.0, BAR);
        for (c, h) in cols
            .iter()
            .zip(["BLOCK", "MHz", "ENSEMBLE", "STATIONS", "MARGIN", "STATE"])
        {
            p.text(
                pos2(table.left() + c, head.center().y),
                Align2::LEFT_CENTER,
                h,
                FontId::monospace(11.0),
                DIM,
            );
        }
        let mut y = head.bottom();
        if rows.is_empty() {
            p.text(
                pos2(table.left() + 16.0, y + 22.0),
                Align2::LEFT_CENTER,
                if running {
                    "sweeping the band…"
                } else {
                    "no ensembles yet"
                },
                FontId::proportional(15.0),
                DIM,
            );
        }
        for TableRow {
            block,
            name,
            stations: n,
            margin,
            state,
            colour,
            current: cur,
        } in rows
        {
            let (b, f) = (block.name, format!("{:.3}", block.hz() / 1e6));
            if y + 44.0 > table.bottom() {
                break;
            }
            let r = Rect::from_min_max(pos2(table.left(), y), pos2(table.right(), y + 44.0));
            if cur {
                p.rect_filled(r, 0.0, FOCUS_BG);
            }
            p.hline(
                r.x_range(),
                r.bottom() - 0.5,
                Stroke::new(1.0, Color32::from_gray(0x26)),
            );
            let fg = if cur { FOCUS } else { TEXT };
            let cy = r.center().y;
            let at = |c: f32| pos2(table.left() + c, cy);
            p.text(
                at(cols[0]),
                Align2::LEFT_CENTER,
                b,
                FontId::monospace(15.0),
                fg,
            );
            p.text(
                at(cols[1]),
                Align2::LEFT_CENTER,
                f,
                FontId::monospace(15.0),
                fg,
            );
            p.text(
                at(cols[2]),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(15.0),
                fg,
            );
            p.text(
                at(cols[3]),
                Align2::LEFT_CENTER,
                n,
                FontId::monospace(15.0),
                fg,
            );
            if let Some(m) = margin {
                let bar = Rect::from_min_size(at(cols[4]) - vec2(0.0, 2.5), vec2(70.0, 5.0));
                p.rect_filled(bar, 0.0, LINE);
                p.rect_filled(
                    Rect::from_min_size(bar.min, vec2(70.0 * (m / 8.0).clamp(0.0, 1.0), 5.0)),
                    0.0,
                    AMBER,
                );
                p.text(
                    at(cols[4] + 80.0),
                    Align2::LEFT_CENTER,
                    format!("{m:+.1} dB"),
                    FontId::monospace(12.0),
                    fg,
                );
            }
            p.text(
                at(cols[5]),
                Align2::LEFT_CENTER,
                state,
                FontId::monospace(13.0),
                colour,
            );
            y += 44.0;
        }

        // The buttons.
        let row = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 34.0), rect.max);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().button_padding = vec2(14.0, 7.0);
        if running {
            if ui
                .button(egui::RichText::new("■ stop scan").color(FOCUS))
                .clicked()
            {
                ask = Some(Ask::StopScan);
            }
        } else {
            if ui
                .button(egui::RichText::new("▶ scan Band III").color(FOCUS))
                .clicked()
            {
                ask = Some(Ask::Scan);
            }
            if ui.button("rescan this block").clicked() {
                ask = Some(Ask::RescanHere);
            }
        }
        ui.add_space(12.0);
        ui.label(egui::RichText::new("list kept in %APPDATA%\\DecDAB\\stations.txt").color(DIM));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n: usize = match &self.simple.scan {
                Some(s) => s.found.iter().map(|m| m.stations.len()).sum(),
                None => self.simple.found.iter().map(|m| m.stations.len()).sum(),
            };
            ui.label(egui::RichText::new(format!("{n} stations")).color(Color32::from_gray(0xdd)));
        });
        ask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_round_trips() {
        let b = parse_block("11D").unwrap();
        let found = vec![Mux {
            block: b,
            name: "Test Ensemble".into(),
            margin_db: Some(3.4),
            stations: vec![
                Station {
                    sid: 0xC0D9,
                    name: "Station A".into(),
                    plus: true,
                },
                Station {
                    sid: 0xC1CE,
                    name: "Station B".into(),
                    plus: false,
                },
            ],
        }];
        let back = parse_list(&list_text(&found));
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].block, b);
        assert_eq!(back[0].name, "Test Ensemble");
        assert_eq!(back[0].margin_db, Some(3.4));
        assert_eq!(back[0].stations.len(), 2);
        assert_eq!(back[0].stations[0].sid, 0xC0D9);
        assert!(back[0].stations[0].plus && !back[0].stations[1].plus);
    }

    #[test]
    fn the_clock_dates() {
        // 2026-11-01 was a Sunday.
        assert_eq!(civil(20_758), (2026, 11, 1));
        assert_eq!((20_758 + 4) % 7, 0);
    }
}
