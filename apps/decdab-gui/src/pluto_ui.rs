// made by CasualArclamp
//! The ADALM-PLUTO / Pluto+ as the GUI keeps it: where it is, its rate, its
//! gain and whether 8-bit IQ is used when the firmware (Tezuka) has it. The
//! driver is `decdab_io::pluto` (DecDVB-T's, by way of DecLAB), receive only.
//!
//! Plain data, whatever the build has: only [`PlutoUi::open`] and the live
//! gain touch the driver, behind the `pluto` feature.

use decdab_io::IqSource;
use eframe::egui::{self, Color32, RichText};

/// The rates on offer, S/s: DAB's own (made by the FPGA's ×8 decimator) and
/// twice it (straight from the AD9361, with room for the receiver's own
/// filter to take the neighbours off).
const RATES: [f64; 2] = [2.048e6, 4.096e6];

#[derive(Debug, Clone, PartialEq)]
pub struct PlutoUi {
    /// `auto`, `usb` or `ip:ADDRESS`.
    pub uri: String,
    pub rate_hz: f64,
    /// Manual gain, dB (about −3 to 71).
    pub gain_db: f64,
    /// The AD9361's slow-attack AGC instead of `gain_db`.
    pub agc: bool,
    /// Refuse Tezuka's 8-bit IQ.
    pub sixteen_bit: bool,
}

impl Default for PlutoUi {
    fn default() -> Self {
        PlutoUi {
            uri: "auto".into(),
            rate_hz: RATES[1],
            gain_db: 50.0,
            agc: false,
            sixteen_bit: false,
        }
    }
}

/// The open Pluto's controls (a build without the driver has none to hold).
#[cfg(feature = "pluto")]
pub type PlutoLive = decdab_io::PlutoControl;
#[cfg(not(feature = "pluto"))]
pub type PlutoLive = ();

impl PlutoUi {
    /// From the preferences: `get` looks a key up.
    pub fn load(get: impl Fn(&str) -> Option<String>) -> PlutoUi {
        let d = PlutoUi::default();
        PlutoUi {
            uri: get("pluto_uri")
                .filter(|u| !u.trim().is_empty() && !u.contains(char::is_whitespace))
                .unwrap_or(d.uri),
            rate_hz: get("pluto_rate")
                .and_then(|r| r.parse().ok())
                .filter(|r| RATES.contains(r))
                .unwrap_or(d.rate_hz),
            gain_db: get("pluto_gain")
                .and_then(|g| g.parse().ok())
                .filter(|g| (-3.0..=71.0).contains(g))
                .unwrap_or(d.gain_db),
            agc: get("pluto_agc").map_or(d.agc, |v| v == "1"),
            sixteen_bit: get("pluto_16bit").map_or(d.sixteen_bit, |v| v == "1"),
        }
    }

    pub fn save(&self) -> Vec<(&'static str, String)> {
        let flag = |on: bool| String::from(if on { "1" } else { "0" });
        vec![
            ("pluto_uri", self.uri.clone()),
            ("pluto_rate", format!("{:.0}", self.rate_hz)),
            ("pluto_gain", format!("{:.0}", self.gain_db)),
            ("pluto_agc", flag(self.agc)),
            ("pluto_16bit", flag(self.sixteen_bit)),
        ]
    }

    #[cfg(feature = "pluto")]
    fn gain(&self) -> decdab_io::PlutoGain {
        if self.agc {
            decdab_io::PlutoGain::Agc
        } else {
            decdab_io::PlutoGain::Manual(self.gain_db)
        }
    }

    /// Open the Pluto at `center_hz`: the source, its controls and what it
    /// said about itself.
    #[cfg(feature = "pluto")]
    pub fn open(&self, center_hz: f64) -> Result<(Box<dyn IqSource>, PlutoLive, String), String> {
        use decdab_io::{EightBit, PlutoSettings, PlutoSource};
        let src = PlutoSource::open(PlutoSettings {
            // Rust note: `PlutoUri` implements `FromStr`, so `parse` turns
            // the text into one (or the reason it cannot).
            uri: self.uri.parse()?,
            center_hz,
            sample_rate: self.rate_hz,
            bandwidth: None,
            gain: self.gain(),
            eight_bit: if self.sixteen_bit {
                EightBit::Off
            } else {
                EightBit::Auto
            },
        })
        .map_err(|e| e.to_string())?;
        let ctl = src.control();
        let i = ctl.info().clone();
        let info = format!(
            "Pluto on {}: {} {}, {}-bit IQ",
            i.link,
            i.model,
            i.firmware,
            if i.eight_bit { 8 } else { 16 }
        );
        Ok((Box::new(src), ctl, info))
    }

    #[cfg(not(feature = "pluto"))]
    pub fn open(&self, _center_hz: f64) -> Result<(Box<dyn IqSource>, PlutoLive, String), String> {
        Err("built without the pluto feature".into())
    }

    /// The settings as one run of controls for a wrapping row; the gain goes
    /// straight to `live`. Returns (open again, save).
    pub fn ui(&mut self, ui: &mut egui::Ui, live: Option<&PlutoLive>) -> (bool, bool) {
        let _ = live;
        let (mut reopen, mut save) = (false, false);
        let r = ui
            .add(egui::TextEdit::singleline(&mut self.uri).desired_width(110.0))
            .on_hover_text(
                "auto: USB (needs ADI's Pluto USB driver, WinUSB), else ip:192.168.2.1; \
                 usb; ip:ADDRESS for a Pluto+ on Ethernet. Receive only.",
            );
        reopen |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.label(RichText::new("rate").color(Color32::GRAY));
        let before = self.rate_hz;
        egui::ComboBox::from_id_salt("pluto_rate")
            .selected_text(format!("{} MS/s", self.rate_hz / 1e6))
            .show_ui(ui, |ui| {
                for r in RATES {
                    ui.selectable_value(&mut self.rate_hz, r, format!("{} MS/s", r / 1e6));
                }
            });
        reopen |= self.rate_hz != before;
        let before = (self.agc, self.gain_db);
        ui.checkbox(&mut self.agc, "AGC")
            .on_hover_text("The AD9361's slow-attack AGC.");
        ui.add_enabled(
            !self.agc,
            egui::DragValue::new(&mut self.gain_db)
                .range(-3.0..=71.0)
                .speed(0.25)
                .max_decimals(0)
                .suffix(" dB"),
        );
        if before != (self.agc, self.gain_db) {
            save = true;
            #[cfg(feature = "pluto")]
            if let Some(c) = live {
                let _ = c.set_gain(self.gain());
            }
        }
        if ui
            .checkbox(&mut self.sixteen_bit, "16-bit")
            .on_hover_text(
                "Stream 16-bit IQ even when the firmware (Tezuka) offers 8-bit, which \
                 halves the link's load.",
            )
            .changed()
        {
            reopen = true;
        }
        (reopen, save || reopen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_the_preferences() {
        let p = PlutoUi {
            uri: "ip:192.168.2.1".into(),
            rate_hz: 2.048e6,
            gain_db: 33.0,
            agc: true,
            sixteen_bit: true,
        };
        let saved = p.save();
        let back = PlutoUi::load(|k| {
            saved
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        });
        assert_eq!(back, p);
        // Nonsense falls back to the defaults.
        let odd = PlutoUi::load(|k| match k {
            "pluto_rate" => Some("3000000".into()),
            "pluto_gain" => Some("200".into()),
            "pluto_uri" => Some("two words".into()),
            _ => None,
        });
        assert_eq!(odd, PlutoUi::default());
    }
}
