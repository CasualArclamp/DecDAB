// made by CasualArclamp
//! Opening what the commands read: an IQ file, or the HackRF tuned to a
//! block or a frequency.

use std::path::Path;

use anyhow::{Result, bail};
use decdab_core::SampleFormat;
use decdab_io::{IqFileReader, IqSource, format_from_path, name_hint};

use crate::Gains;

/// The HackRF's rate for live work: twice DAB's, so one halfband stage
/// cleans the neighbours off.
pub const LIVE_RATE_MSPS: f64 = 4.096;

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
    #[cfg(feature = "hackrf")]
    {
        Ok(Box::new(crate::open(hz, LIVE_RATE_MSPS, gains)?))
    }
    #[cfg(not(feature = "hackrf"))]
    {
        let _ = (hz, gains);
        bail!("{what}: no such file, and built without the hackrf feature")
    }
}

#[allow(dead_code)]
fn _bail_used() -> Result<()> {
    bail!("")
}
