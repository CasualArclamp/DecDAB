// made by CasualArclamp
//! Opening a source and running the engine on it: the HackRF, the Airspy or
//! an RTL-SDR at a block, or a recorded IQ file.

use std::path::PathBuf;

use decdab_core::SampleFormat;
use decdab_engine::{Rx, RxOptions};
use decdab_io::{IqFileReader, IqSource, format_from_path};

/// The Airspy's three gain stages (indices, with libairspy's presets).
pub use decdab_io::AirspyGains;

/// The RTL-SDR's tuner gain (its AGC, or a fixed step).
pub use decdab_io::RtlGains;

/// The HackRF's three gain stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gains {
    /// RF amplifier, +14 dB.
    pub amp: bool,
    /// 0–40 dB in 8 dB steps.
    pub lna: u16,
    /// 0–62 dB in 2 dB steps.
    pub vga: u16,
}

impl Default for Gains {
    fn default() -> Self {
        // What receives Band III on the author's roof (a UHF aerial): the
        // RF amplifier on, LNA 40, VGA 20 (VGA 30 clips the strongest
        // ensemble).
        Gains {
            amp: true,
            lna: 40,
            vga: 20,
        }
    }
}

/// What to open.
#[derive(Debug, Clone)]
pub enum Spec {
    HackRf {
        center_hz: f64,
        rate_hz: f64,
        gains: Gains,
    },
    Airspy {
        center_hz: f64,
        /// The rate delivered (2.048, 4.096 or 8.192 MS/s), not the radio's.
        rate_hz: f64,
        gains: AirspyGains,
    },
    /// Always 2.048 MS/s.
    RtlSdr {
        center_hz: f64,
        gains: RtlGains,
    },
    File {
        path: PathBuf,
        center_hz: f64,
        rate_hz: f64,
    },
}

type GainSetter = Box<dyn Fn(Gains) -> Result<(), String>>;
type AirGainSetter = Box<dyn Fn(AirspyGains) -> Result<(), String>>;
type Setter<T> = Box<dyn Fn(T) -> Result<(), String>>;

/// A running source.
pub struct Session {
    pub rx: Rx,
    pub live: bool,
    set_gains: Option<GainSetter>,
    set_air_gains: Option<AirGainSetter>,
    set_rtl_gains: Option<Setter<RtlGains>>,
    set_filter: Option<Setter<Option<f32>>>,
    set_ppm: Option<Setter<f32>>,
}

impl Session {
    /// `sound`: play the chosen service on the sound card.
    pub fn open(spec: &Spec, sound: bool) -> Result<Session, String> {
        let opts = RxOptions {
            sound,
            ..RxOptions::default()
        };
        match spec {
            #[cfg(feature = "hackrf")]
            Spec::HackRf {
                center_hz,
                rate_hz,
                gains,
            } => {
                use decdab_io::{HackRfGains, HackRfSettings, HackRfSource};
                let src = HackRfSource::open(HackRfSettings {
                    center_hz: center_hz.round() as u64,
                    sample_rate: rate_hz.round() as u32,
                    gains: HackRfGains {
                        amp: gains.amp,
                        lna_db: gains.lna,
                        vga_db: gains.vga,
                    },
                })
                .map_err(|e| e.to_string())?;
                let ctl = src.control();
                let probe = ctl.clone();
                let (filter_ctl, ppm_ctl) = (ctl.clone(), ctl.clone());
                let rx = Rx::start(
                    Box::new(src),
                    opts,
                    Some(Box::new(move || probe.overflows())),
                );
                Ok(Session {
                    rx,
                    live: true,
                    set_air_gains: None,
                    set_rtl_gains: None,
                    set_gains: Some(Box::new(move |g| {
                        ctl.set_gains(decdab_io::HackRfGains {
                            amp: g.amp,
                            lna_db: g.lna,
                            vga_db: g.vga,
                        })
                        .map_err(|e| e.to_string())
                    })),
                    set_filter: Some(Box::new(move |mhz| {
                        filter_ctl
                            .set_filter_bandwidth(mhz.map(|f| (f * 1e6) as u32))
                            .map_err(|e| e.to_string())
                    })),
                    set_ppm: Some(Box::new(move |ppm| {
                        ppm_ctl.set_ppm(f64::from(ppm)).map_err(|e| e.to_string())
                    })),
                })
            }
            #[cfg(not(feature = "hackrf"))]
            Spec::HackRf { .. } => Err("built without the hackrf feature".into()),
            #[cfg(feature = "airspy")]
            Spec::Airspy {
                center_hz,
                rate_hz,
                gains,
            } => {
                use decdab_io::{AirspySettings, AirspySource};
                let src = AirspySource::open(AirspySettings {
                    center_hz: center_hz.round() as u64,
                    sample_rate: rate_hz.round() as u32,
                    gains: *gains,
                    packed: false,
                })
                .map_err(|e| e.to_string())?;
                let ctl = src.control();
                let probe = ctl.clone();
                let rx = Rx::start(
                    Box::new(src),
                    opts,
                    Some(Box::new(move || probe.overflows())),
                );
                // The HackRF's baseband filter and frequency correction
                // have no counterpart: the Airspy has neither to set.
                Ok(Session {
                    rx,
                    live: true,
                    set_gains: None,
                    set_air_gains: Some(Box::new(move |g| {
                        ctl.set_gains(g).map_err(|e| e.to_string())
                    })),
                    set_rtl_gains: None,
                    set_filter: None,
                    set_ppm: None,
                })
            }
            #[cfg(not(feature = "airspy"))]
            Spec::Airspy { .. } => Err("built without the airspy feature".into()),
            #[cfg(feature = "rtlsdr")]
            Spec::RtlSdr { center_hz, gains } => {
                use decdab_io::{RtlSdrSource, RtlSettings};
                // The frequency correction follows from the SDR window, as
                // the HackRF's does.
                let src = RtlSdrSource::open(RtlSettings {
                    center_hz: center_hz.round() as u64,
                    gains: *gains,
                    ..RtlSettings::default()
                })
                .map_err(|e| e.to_string())?;
                let ctl = src.control();
                let (probe, ppm_ctl) = (ctl.clone(), ctl.clone());
                let rx = Rx::start(
                    Box::new(src),
                    opts,
                    Some(Box::new(move || probe.overflows())),
                );
                // The tuner's IF filter follows the rate: no baseband filter
                // to choose.
                Ok(Session {
                    rx,
                    live: true,
                    set_gains: None,
                    set_air_gains: None,
                    set_rtl_gains: Some(Box::new(move |g| {
                        ctl.set_gains(g).map_err(|e| e.to_string())
                    })),
                    set_filter: None,
                    set_ppm: Some(Box::new(move |ppm| {
                        ppm_ctl.set_ppm(f64::from(ppm)).map_err(|e| e.to_string())
                    })),
                })
            }
            #[cfg(not(feature = "rtlsdr"))]
            Spec::RtlSdr { .. } => Err("built without the rtlsdr feature".into()),
            Spec::File {
                path,
                center_hz,
                rate_hz,
            } => {
                let fmt = format_from_path(path).unwrap_or(SampleFormat::Cs8);
                let src = IqFileReader::open(path, fmt, *rate_hz, 1 << 15)
                    .map_err(|e| format!("cannot open {}: {e}", path.display()))?
                    .with_center_freq(*center_hz);
                let src: Box<dyn IqSource> = Box::new(src);
                Ok(Session {
                    rx: Rx::start(src, opts, None),
                    live: false,
                    set_gains: None,
                    set_air_gains: None,
                    set_rtl_gains: None,
                    set_filter: None,
                    set_ppm: None,
                })
            }
        }
    }

    /// The HackRF's baseband filter, MHz (`None`: automatic); no-op for a file.
    pub fn set_filter_mhz(&self, mhz: Option<f32>) -> Result<(), String> {
        self.set_filter.as_ref().map_or(Ok(()), |f| f(mhz))
    }

    /// The HackRF's or RTL-SDR's frequency error, ppm; no-op for a file.
    pub fn set_ppm(&self, ppm: f32) -> Result<(), String> {
        self.set_ppm.as_ref().map_or(Ok(()), |f| f(ppm))
    }

    /// Change the Airspy's gains (no-op for anything else).
    pub fn set_air_gains(&self, g: AirspyGains) -> Result<(), String> {
        self.set_air_gains.as_ref().map_or(Ok(()), |f| f(g))
    }

    /// Change the RTL-SDR's gain (no-op for anything else).
    pub fn set_rtl_gains(&self, g: RtlGains) -> Result<(), String> {
        self.set_rtl_gains.as_ref().map_or(Ok(()), |f| f(g))
    }

    /// Change the HackRF's gains (no-op for anything else).
    pub fn set_gains(&self, g: Gains) -> Result<(), String> {
        match &self.set_gains {
            Some(f) => f(g),
            None => Ok(()),
        }
    }
}
