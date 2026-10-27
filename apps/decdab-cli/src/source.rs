// made by CasualArclamp
//! Opening what the commands read: an IQ file, or a radio (the HackRF, the
//! Airspy or an RTL-SDR) tuned to a block or a frequency.

use std::path::Path;

#[allow(unused_imports)] // `bail` is only used without a radio feature
use anyhow::{Result, bail};
use decdab_core::{Iq, SampleFormat};
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};

use crate::{Gains, Radio};

/// The radio's output rate for live work: twice DAB's, so one halfband stage
/// cleans the neighbours off. (The Airspy runs at 20 MS/s and is brought
/// down to this.)
pub const LIVE_RATE_MSPS: f64 = 4.096;

/// A live radio, whichever one: an [`IqSource`] that also counts the buffers
/// it dropped.
pub struct Live {
    src: Box<dyn IqSource>,
    overflows: Box<dyn Fn() -> u64 + Send>,
}

impl Live {
    /// Buffers dropped because the reader fell behind.
    pub fn overflows(&self) -> u64 {
        (self.overflows)()
    }

    pub fn into_source(self) -> Box<dyn IqSource> {
        self.src
    }
}

impl IqSource for Live {
    fn read(&mut self, out: &mut Vec<Iq>) -> decdab_core::Result<usize> {
        self.src.read(out)
    }
    fn sample_rate(&self) -> f64 {
        self.src.sample_rate()
    }
    fn center_freq(&self) -> f64 {
        self.src.center_freq()
    }
    fn describe(&self) -> String {
        self.src.describe()
    }
    fn retune(&mut self, hz: f64) -> decdab_core::Result<bool> {
        self.src.retune(hz)
    }
    fn is_live(&self) -> bool {
        self.src.is_live()
    }
}

/// The rate a radio will really run at when `rate` (MS/s) is asked for: the
/// RTL-SDR runs at 2.048 only (its USB side tops out near 3.2 MS/s).
pub fn live_rate(radio: Radio, rate: f64) -> f64 {
    match radio {
        Radio::Rtlsdr => 2.048,
        _ => rate,
    }
}

/// Open the chosen radio at `hz` (Hz) and `rate` (MS/s).
pub fn open_live(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    match g.radio {
        Radio::Hackrf => open_hackrf(hz, rate, g),
        Radio::Airspy => open_airspy(hz, rate, g),
        Radio::Rtlsdr => open_rtlsdr(hz, g),
    }
}

#[cfg(feature = "hackrf")]
fn open_hackrf(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{HackRfGains, HackRfSettings, HackRfSource};
    let src = HackRfSource::open(HackRfSettings {
        center_hz: hz.round() as u64,
        sample_rate: (rate * 1e6).round() as u32,
        gains: HackRfGains {
            amp: g.amp,
            lna_db: g.lna,
            vga_db: g.vga,
        },
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
}

#[cfg(not(feature = "hackrf"))]
fn open_hackrf(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the hackrf feature")
}

#[cfg(feature = "airspy")]
fn open_airspy(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{AirspySettings, AirspySource};
    let src = AirspySource::open(AirspySettings {
        center_hz: hz.round() as u64,
        sample_rate: (rate * 1e6).round() as u32,
        gains: g.airspy_gains(),
        packed: g.airspy_packed,
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
}

#[cfg(not(feature = "airspy"))]
fn open_airspy(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the airspy feature")
}

#[cfg(feature = "rtlsdr")]
fn open_rtlsdr(hz: f64, g: Gains) -> Result<Live> {
    use decdab_io::{RtlSdrSource, RtlSettings};
    let src = RtlSdrSource::open(RtlSettings {
        center_hz: hz.round() as u64,
        gains: g.rtl_gain,
        ppm: g.rtl_ppm,
        ..RtlSettings::default()
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
}

#[cfg(not(feature = "rtlsdr"))]
fn open_rtlsdr(_: f64, _: Gains) -> Result<Live> {
    bail!("built without the rtlsdr feature")
}

pub fn open(what: &str, gains: Gains) -> Result<Box<dyn IqSource>> {
    let p = Path::new(what);
    if p.exists() {
        let hint = name_hint(p);
        let fmt = format_from_path(p).unwrap_or(SampleFormat::Cs8);
        let rate = hint.sample_rate.unwrap_or(2.048e6);
        let r = IqFileReader::open(p, fmt, rate, 1 << 16)?
            .with_center_freq(hint.center_hz.unwrap_or(0.0));
        return Ok(Box::new(r));
    }
    let hz = crate::tuning(what)?;
    Ok(open_live(hz, live_rate(gains.radio, LIVE_RATE_MSPS), gains)?.into_source())
}
