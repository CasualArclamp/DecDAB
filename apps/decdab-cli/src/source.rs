// made by CasualArclamp
//! Opening what the commands read: an IQ file, or a radio (the HackRF, the
//! Airspy, an RTL-SDR, the Pluto, a Fobos SDR, an SDRplay RSP, a USRP, a
//! radio behind a DecSAT server, or network IQ) tuned to a block or a
//! frequency.

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
        // The Fobos starts at 8 MS/s, and the SDRplay at 2: DAB's own rates
        // are not among theirs, and the receiver resamples whatever comes.
        Radio::Fobos => rate.max(8.0),
        Radio::Sdrplay => rate.max(2.0),
        _ => rate,
    }
}

/// Open the chosen radio at `hz` (Hz) and `rate` (MS/s).
pub fn open_live(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    match g.radio {
        Radio::Hackrf => open_hackrf(hz, rate, g),
        Radio::Airspy => open_airspy(hz, rate, g),
        Radio::Rtlsdr => open_rtlsdr(hz, g),
        Radio::Pluto => open_pluto(hz, rate, g),
        Radio::Fobos => open_fobos(hz, rate, g),
        Radio::Sdrplay => open_sdrplay(hz, rate, g),
        Radio::Uhd => open_uhd(hz, rate, g),
        Radio::Remote => open_remote(hz, rate, g),
        Radio::Net => open_net(hz, rate, g),
    }
}

#[cfg(feature = "pluto")]
fn open_pluto(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{EightBit, PlutoGain, PlutoSettings, PlutoSource};
    let gain = if g.pluto_gain.eq_ignore_ascii_case("agc") {
        PlutoGain::Agc
    } else {
        PlutoGain::Manual(g.pluto_gain.parse().map_err(|_| {
            anyhow::anyhow!("--pluto-gain takes dB or agc, not {:?}", g.pluto_gain)
        })?)
    };
    let src = PlutoSource::open(PlutoSettings {
        uri: g.pluto_uri.parse().map_err(anyhow::Error::msg)?,
        center_hz: hz,
        sample_rate: rate * 1e6,
        bandwidth: None,
        gain,
        eight_bit: if g.pluto_16bit {
            EightBit::Off
        } else {
            EightBit::Auto
        },
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.lost()),
    })
}

#[cfg(not(feature = "pluto"))]
fn open_pluto(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the pluto feature")
}

/// `A,B` as two numbers.
#[cfg(any(feature = "fobos", feature = "sdrplay"))]
fn gain_pair(s: &str, what: &str) -> Result<(u8, u8)> {
    let bad = || anyhow::anyhow!("{what}, not {s:?}");
    let (a, b) = s.split_once(',').ok_or_else(bad)?;
    Ok((
        a.trim().parse().map_err(|_| bad())?,
        b.trim().parse().map_err(|_| bad())?,
    ))
}

#[cfg(feature = "fobos")]
fn open_fobos(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{FobosGains, FobosSettings, FobosSource};
    let d = FobosSettings::default();
    let gains = match &g.fobos_gain {
        Some(s) => {
            let (lna, vga) = gain_pair(s, "--fobos-gain takes LNA,VGA (0-3, 0-31)")?;
            FobosGains { lna, vga }.snapped()
        }
        None => d.gains,
    };
    let src = FobosSource::open(FobosSettings {
        center_hz: hz.round() as u64,
        sample_rate: (rate * 1e6).round() as u32,
        gains,
        external_clock: g.fobos_ext_clock,
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
}

#[cfg(not(feature = "fobos"))]
fn open_fobos(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the fobos feature")
}

#[cfg(feature = "sdrplay")]
fn open_sdrplay(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{SdrplayGains, SdrplaySettings, SdrplaySource};
    let d = SdrplaySettings::default();
    let gains = match &g.sdrplay_gain {
        Some(s) => {
            let (lna, gr) = gain_pair(s, "--sdrplay-gain takes LNASTATE,IFGR (0-9, 20-59)")?;
            SdrplayGains { lna, gr }.snapped()
        }
        None => d.gains,
    };
    let src = SdrplaySource::open(SdrplaySettings {
        center_hz: hz.round() as u64,
        sample_rate: (rate * 1e6).round() as u32,
        gains,
        tuner_b: g.sdrplay_tuner_b,
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
}

#[cfg(not(feature = "sdrplay"))]
fn open_sdrplay(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the sdrplay feature")
}

#[cfg(all(windows, feature = "uhd"))]
fn open_uhd(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use decdab_io::{UhdSettings, UhdSource};
    let src = UhdSource::open(UhdSettings {
        device_args: g.uhd_args.clone(),
        center_hz: hz,
        rate: rate * 1e6,
        gain: g.uhd_gain,
        antenna: g.uhd_antenna.clone(),
        ..UhdSettings::default()
    })?;
    // UHD's overflows are counted inside the driver and end the stream if
    // they do not stop; there is no counter to read here.
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(|| 0),
    })
}

#[cfg(not(all(windows, feature = "uhd")))]
fn open_uhd(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the uhd feature (uhd.dll is loaded on Windows only)")
}

#[cfg(feature = "remote")]
fn open_remote(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use anyhow::Context;
    use decdab_io::remote::{ConnectOptions, IqParams, RadioRequest, RemoteIqSource};
    let password = match &g.remote_password_file {
        Some(f) => Some(
            std::fs::read_to_string(f)
                .with_context(|| format!("--remote-password-file {}", f.display()))?
                .trim()
                .to_string(),
        ),
        None => std::env::var("DECDAB_REMOTE_PASSWORD").ok(),
    };
    let mut stages = Vec::new();
    for part in g
        .remote_gain
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|p| !p.is_empty())
    {
        let (name, v) = part
            .split_once('=')
            .context("--remote-gain takes NAME=VALUE,NAME=VALUE")?;
        let v: f64 = v
            .parse()
            .with_context(|| format!("--remote-gain {part}"))?;
        stages.push((name.to_string(), v));
    }
    let src = RemoteIqSource::connect(
        &ConnectOptions {
            addr: g
                .remote
                .clone()
                .context("--radio remote: give --remote HOST[:PORT]")?,
            name: "decdab".into(),
            password,
            takeover: false,
        },
        RadioRequest {
            key: g.remote_radio.clone(),
            center_hz: hz,
            sample_rate: rate * 1e6,
            gains: stages,
        },
        IqParams {
            bits: g.remote_bits,
            zstd: if g.remote_zstd { 1 } else { 0 },
            slice: None,
        },
    )?;
    let lost = src.dropped_counter();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || lost.load(std::sync::atomic::Ordering::Relaxed)),
    })
}

#[cfg(not(feature = "remote"))]
fn open_remote(_: f64, _: f64, _: Gains) -> Result<Live> {
    bail!("built without the remote feature")
}

fn open_net(hz: f64, rate: f64, g: Gains) -> Result<Live> {
    use anyhow::Context;
    use decdab_io::{NetIqSettings, NetIqSource, NetSample};
    let url = g
        .net
        .as_deref()
        .context("--radio net: give --net tcp://HOST:PORT (or listen://, udp://)")?;
    let (mode, addr) = NetIqSettings::parse_url(url)?;
    let src = NetIqSource::open(NetIqSettings {
        mode,
        addr,
        sample: NetSample::parse(&g.net_iq)
            .with_context(|| format!("--net-iq {}: int8, uint8, int16, int32 or float32", g.net_iq))?,
        sample_rate: rate * 1e6,
        center_hz: hz,
    })?;
    let ctl = src.control();
    Ok(Live {
        src: Box::new(src),
        overflows: Box::new(move || ctl.overflows()),
    })
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
