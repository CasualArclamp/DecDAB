// made by CasualArclamp
//! The Airspy R2's USB protocol as plain data: what to send, in what order,
//! and how to read what comes back. Nothing here touches USB, so every
//! command, the gain tables and the sample unpacker are unit-tested without a
//! radio; `airspy.rs` (feature `airspy`) is the thin layer that sends them.
//!
//! The requests, their encodings, the gain tables and the packed-sample layout
//! follow `airspy.c` and `airspy_commands.h` of libairspy (airspyone_host,
//! Copyright (c) 2013-2016 Benjamin Vernoux, Ian Gilmour, Youssef Touil and
//! others; BSD-3-Clause), re-implemented here rather than linked.
//!
//! **Antenna-port power (the bias tee) is never switched on.** The one
//! command that could do it, `GPIO_WRITE`, is built by [`bias_tee_off`] alone,
//! which hard-codes the value 0, and it is sent at open and at every start.
//! Putting DC on the antenna port of whatever is connected (a filter, an
//! LNA, an LNB) can damage it, so DecDAB will not. A test checks every
//! command sequence for this, and that `airspy.rs` does not mention the
//! request at all.

use decdab_core::{Error, Result};

/// Airspy's USB IDs (Airspy / Airspy R2 / Mini all use these).
pub const VENDOR_ID: u16 = 0x1D50;
pub const PRODUCT_ID: u16 = 0x60A1;
/// The bulk IN endpoint the samples arrive on.
pub const ENDPOINT_IN: u8 = 0x81;

/// The IQ rate the R2 is run at: the ADC samples at twice this, 20 MS/s, and
/// the bulk stream is that real signal (libairspy's `iqconverter` makes the
/// 10 MS/s complex stream from it).
pub const IQ_RATE_HZ: u32 = 10_000_000;

/// The tuner's range (libairspy `airspy_set_freq`).
pub const FREQ_MIN_HZ: u64 = 24_000_000;
pub const FREQ_MAX_HZ: u64 = 1_800_000_000;

/// Bytes per bulk transfer. Unpacked: 131 072 samples of 16 bits, ~6.5 ms.
/// Packed: 98 304 samples of 12 bits, ~4.9 ms. Both hold a whole number of
/// 8-sample groups (16 and 12 bytes).
pub const TRANSFER_BYTES: usize = 1 << 18;
pub const TRANSFER_BYTES_PACKED: usize = 147_456;

/// Requests (`airspy_vendor_request`). All are `Vendor | Device`.
pub mod request {
    pub const RECEIVER_MODE: u8 = 1;
    pub const BOARD_ID_READ: u8 = 9;
    pub const VERSION_STRING_READ: u8 = 10;
    pub const SET_SAMPLERATE: u8 = 12;
    pub const SET_FREQ: u8 = 13;
    pub const SET_LNA_GAIN: u8 = 14;
    pub const SET_MIXER_GAIN: u8 = 15;
    pub const SET_VGA_GAIN: u8 = 16;
    pub const SET_LNA_AGC: u8 = 17;
    pub const SET_MIXER_AGC: u8 = 18;
    /// Drives a GPIO pin. Used for one thing only: switching the bias tee
    /// OFF. See the module comment.
    pub const GPIO_WRITE: u8 = 21;
    pub const GET_SAMPLERATES: u8 = 25;
    pub const SET_PACKING: u8 = 26;
}

/// A control transfer, as the arguments of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbCommand {
    pub request: u8,
    pub value: u16,
    pub index: u16,
    pub data: CommandData,
}

/// The data stage: bytes to send (an OUT transfer, possibly none), or the
/// number of bytes to read (an IN transfer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandData {
    Out(Vec<u8>),
    In(u16),
}

impl UsbCommand {
    fn out(request: u8, value: u16, index: u16, data: &[u8]) -> Self {
        UsbCommand {
            request,
            value,
            index,
            data: CommandData::Out(data.to_vec()),
        }
    }

    /// libairspy's "set" calls are IN transfers of one byte whose value is
    /// ignored, but whose length is checked.
    fn set(request: u8, index: u16) -> Self {
        UsbCommand {
            request,
            value: 0,
            index,
            data: CommandData::In(1),
        }
    }
}

/// `airspy_set_receiver_mode`: 0 off, 1 receive.
pub fn receiver_mode(on: bool) -> UsbCommand {
    UsbCommand::out(request::RECEIVER_MODE, on as u16, 0, &[])
}

/// `airspy_set_freq`: the frequency as a little-endian u32, in Hz.
pub fn set_freq(hz: u64) -> Result<UsbCommand> {
    if !(FREQ_MIN_HZ..=FREQ_MAX_HZ).contains(&hz) {
        return Err(Error::Other(format!(
            "Airspy: {:.3} MHz is outside the tuner's {}–{} MHz",
            hz as f64 / 1e6,
            FREQ_MIN_HZ / 1_000_000,
            FREQ_MAX_HZ / 1_000_000
        )));
    }
    Ok(UsbCommand::out(
        request::SET_FREQ,
        0,
        0,
        &(hz as u32).to_le_bytes(),
    ))
}

/// `airspy_set_samplerate`: the index into the device's list of rates.
pub fn set_sample_rate(index: u16) -> UsbCommand {
    UsbCommand::set(request::SET_SAMPLERATE, index)
}

/// `airspy_set_packing`: 12-bit samples packed 8 to 12 bytes.
pub fn set_packing(packed: bool) -> UsbCommand {
    UsbCommand::set(request::SET_PACKING, packed as u16)
}

pub fn set_lna_agc(on: bool) -> UsbCommand {
    UsbCommand::set(request::SET_LNA_AGC, on as u16)
}

pub fn set_mixer_agc(on: bool) -> UsbCommand {
    UsbCommand::set(request::SET_MIXER_AGC, on as u16)
}

/// The three gain commands for `gains` (clamped to the hardware's ranges
/// first).
pub fn set_gains(gains: AirspyGains) -> [UsbCommand; 3] {
    let g = gains.clamped();
    [
        UsbCommand::set(request::SET_LNA_GAIN, g.lna as u16),
        UsbCommand::set(request::SET_MIXER_GAIN, g.mixer as u16),
        UsbCommand::set(request::SET_VGA_GAIN, g.vga as u16),
    ]
}

/// The bias tee, OFF. The pin is port 1, pin 13: `(port << 5) | pin`, as in
/// libairspy's `airspy_gpio_write`; the value (`wValue`) is 0. This is the
/// only place a `GPIO_WRITE` is built, and there is deliberately no
/// parameter.
pub fn bias_tee_off() -> UsbCommand {
    const GPIO_PORT1: u16 = 1;
    const GPIO_PIN13: u16 = 13;
    UsbCommand::out(request::GPIO_WRITE, 0, (GPIO_PORT1 << 5) | GPIO_PIN13, &[])
}

pub fn read_board_id() -> UsbCommand {
    UsbCommand {
        request: request::BOARD_ID_READ,
        value: 0,
        index: 0,
        data: CommandData::In(1),
    }
}

pub fn read_version() -> UsbCommand {
    UsbCommand {
        request: request::VERSION_STRING_READ,
        value: 0,
        index: 0,
        data: CommandData::In(127),
    }
}

/// `airspy_get_samplerates`, step 1: with `wIndex` 0, the number of rates.
pub fn read_sample_rate_count() -> UsbCommand {
    UsbCommand {
        request: request::GET_SAMPLERATES,
        value: 0,
        index: 0,
        data: CommandData::In(4),
    }
}

/// Step 2: with `wIndex` the count, that many rates.
pub fn read_sample_rates(count: u32) -> UsbCommand {
    UsbCommand {
        request: request::GET_SAMPLERATES,
        value: 0,
        index: count as u16,
        data: CommandData::In((count * 4) as u16),
    }
}

/// The count from step 1's reply.
pub fn parse_sample_rate_count(reply: &[u8]) -> Result<u32> {
    let b: [u8; 4] = reply
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| Error::Other("Airspy: short reply to the sample-rate count".into()))?;
    let n = u32::from_le_bytes(b);
    // The R2 has two; a wild number is a bad reply, not a reason to ask for
    // gigabytes.
    if (1..=32).contains(&n) {
        Ok(n)
    } else {
        Err(Error::Other(format!(
            "Airspy: implausible sample-rate count {n}"
        )))
    }
}

/// The rates from step 2's reply, Hz.
pub fn parse_sample_rates(reply: &[u8], count: u32) -> Result<Vec<u32>> {
    let want = count as usize * 4;
    if reply.len() < want {
        return Err(Error::Other(
            "Airspy: short reply to the sample-rate list".into(),
        ));
    }
    Ok(reply[..want]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect())
}

/// The index of the 10 MS/s IQ rate in the device's list (the R2 lists
/// 10 000 000 and 2 500 000).
pub fn iq_rate_index(rates: &[u32]) -> Result<u16> {
    rates
        .iter()
        .position(|&r| r == IQ_RATE_HZ)
        .map(|i| i as u16)
        .ok_or_else(|| {
            Error::Other(format!(
                "Airspy: no 10 MS/s sample rate (the device offers {rates:?})"
            ))
        })
}

/// The board's name from `BOARD_ID_READ` (libairspy `airspy_board_id_name`).
pub fn board_name(id: u8) -> &'static str {
    match id {
        0 => "AIRSPY",
        0xFF => "INVALID",
        _ => "unknown board",
    }
}

/// The firmware string from `VERSION_STRING_READ`: text, possibly with a
/// terminating NUL.
pub fn parse_version(reply: &[u8]) -> String {
    let end = reply.iter().position(|&b| b == 0).unwrap_or(reply.len());
    String::from_utf8_lossy(&reply[..end]).trim().to_string()
}

// ---- gains ------------------------------------------------------------

/// The R820T's three gain stages as the Airspy sets them: indices, not dB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AirspyGains {
    /// 0–14.
    pub lna: u8,
    /// 0–15.
    pub mixer: u8,
    /// 0–15.
    pub vga: u8,
}

pub const LNA_MAX: u8 = 14;
pub const MIXER_MAX: u8 = 15;
pub const VGA_MAX: u8 = 15;
/// The presets' range: 0 (least gain) to 21.
pub const PRESET_MAX: u8 = 21;

/// libairspy's `linearity_gain_*` tables (airspy.c), best linearity for each
/// total gain; index 0 is the most gain.
const LINEARITY_VGA: [u8; 22] = [
    13, 12, 11, 11, 11, 11, 11, 10, 10, 10, 10, 10, 10, 10, 10, 10, 9, 8, 7, 6, 5, 4,
];
const LINEARITY_MIXER: [u8; 22] = [
    12, 12, 11, 9, 8, 7, 6, 6, 5, 0, 0, 1, 0, 0, 2, 2, 1, 1, 1, 1, 0, 0,
];
const LINEARITY_LNA: [u8; 22] = [
    14, 14, 14, 13, 12, 10, 9, 9, 8, 9, 8, 6, 5, 3, 1, 0, 0, 0, 0, 0, 0, 0,
];
/// libairspy's `sensitivity_gain_*` tables: best noise figure for each total
/// gain.
const SENSITIVITY_VGA: [u8; 22] = [
    13, 12, 11, 10, 9, 8, 7, 6, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4,
];
const SENSITIVITY_MIXER: [u8; 22] = [
    12, 12, 12, 12, 11, 10, 10, 9, 9, 8, 7, 4, 4, 4, 3, 2, 2, 1, 0, 0, 0, 0,
];
const SENSITIVITY_LNA: [u8; 22] = [
    14, 14, 14, 14, 14, 14, 14, 14, 14, 13, 12, 12, 9, 9, 8, 7, 6, 5, 3, 2, 1, 0,
];

impl AirspyGains {
    /// Into the hardware's ranges (an out-of-range value is not sent).
    pub fn clamped(self) -> Self {
        AirspyGains {
            lna: self.lna.min(LNA_MAX),
            mixer: self.mixer.min(MIXER_MAX),
            vga: self.vga.min(VGA_MAX),
        }
    }

    /// libairspy's `airspy_set_linearity_gain(n)`: `n` 0 (least gain) to 21
    /// (most), stored from the most gain down, so the table index is 21 − n.
    pub fn linearity(n: u8) -> Self {
        let i = (PRESET_MAX - n.min(PRESET_MAX)) as usize;
        AirspyGains {
            lna: LINEARITY_LNA[i],
            mixer: LINEARITY_MIXER[i],
            vga: LINEARITY_VGA[i],
        }
    }

    /// libairspy's `airspy_set_sensitivity_gain(n)`.
    pub fn sensitivity(n: u8) -> Self {
        let i = (PRESET_MAX - n.min(PRESET_MAX)) as usize;
        AirspyGains {
            lna: SENSITIVITY_LNA[i],
            mixer: SENSITIVITY_MIXER[i],
            vga: SENSITIVITY_VGA[i],
        }
    }

    /// `linearity:N` or `sensitivity:N` (N 0–21), for a command line.
    pub fn parse_preset(text: &str) -> std::result::Result<Self, String> {
        let (kind, n) = text
            .split_once(':')
            .ok_or_else(|| format!("{text}: expected linearity:N or sensitivity:N"))?;
        let n: u8 = n
            .trim()
            .parse()
            .ok()
            .filter(|n| *n <= PRESET_MAX)
            .ok_or_else(|| format!("{text}: N must be 0–{PRESET_MAX}"))?;
        match kind.trim().to_ascii_lowercase().as_str() {
            "linearity" => Ok(Self::linearity(n)),
            "sensitivity" => Ok(Self::sensitivity(n)),
            _ => Err(format!("{text}: expected linearity:N or sensitivity:N")),
        }
    }
}

impl Default for AirspyGains {
    /// A high-gain sensitivity preset (15): Band III is weak on the author's
    /// aerial (the HackRF wants its amp, LNA 40 and VGA 20 there). Untested on
    /// a real Airspy.
    fn default() -> Self {
        Self::sensitivity(15)
    }
}

// ---- settings and the start sequence -------------------------------------

/// What a live session is set to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirspySettings {
    pub center_hz: u64,
    /// The rate the source delivers, Hz: 2.048, 4.096 or 8.192 MS/s (what the
    /// engine takes), made from the Airspy's 20 MS/s by `airspy_dsp`.
    pub sample_rate: u32,
    pub gains: AirspyGains,
    /// 12-bit packed transfers (a third less USB traffic: 30 instead of
    /// 40 MB/s). Off by default.
    pub packed: bool,
}

impl Default for AirspySettings {
    fn default() -> Self {
        AirspySettings {
            center_hz: 225_648_000, // 12B
            sample_rate: 4_096_000,
            gains: AirspyGains::default(),
            packed: false,
        }
    }
}

impl AirspySettings {
    pub fn transfer_bytes(&self) -> usize {
        if self.packed {
            TRANSFER_BYTES_PACKED
        } else {
            TRANSFER_BYTES
        }
    }
}

/// A step of starting the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Command(UsbCommand),
    /// `CLEAR_FEATURE(ENDPOINT_HALT)` on the bulk endpoint.
    ClearHalt,
}

/// Everything sent to bring the stream up, in libairspy's order
/// (`airspy_open`, `airspy_set_*`, `airspy_start_rx`): bias tee off, rate,
/// packing, frequency, both AGCs off, gains, receiver off, clear the halt,
/// receiver on. `rate_index` is [`iq_rate_index`]'s.
pub fn start_sequence(s: &AirspySettings, rate_index: u16) -> Result<Vec<Step>> {
    let mut v = vec![
        Step::Command(bias_tee_off()),
        Step::ClearHalt,
        Step::Command(set_sample_rate(rate_index)),
        Step::Command(set_packing(s.packed)),
        Step::Command(set_freq(s.center_hz)?),
        Step::Command(set_lna_agc(false)),
        Step::Command(set_mixer_agc(false)),
    ];
    v.extend(set_gains(s.gains).into_iter().map(Step::Command));
    v.extend([
        Step::Command(receiver_mode(false)),
        Step::ClearHalt,
        Step::Command(receiver_mode(true)),
    ]);
    Ok(v)
}

// ---- packed samples ---------------------------------------------------------

/// Eight 12-bit samples from 12 bytes (three little-endian u32 words), as
/// libairspy's `unpack_samples`.
pub fn unpack8(group: &[u8; 12]) -> [u16; 8] {
    let w0 = u32::from_le_bytes([group[0], group[1], group[2], group[3]]);
    let w1 = u32::from_le_bytes([group[4], group[5], group[6], group[7]]);
    let w2 = u32::from_le_bytes([group[8], group[9], group[10], group[11]]);
    [
        ((w0 >> 20) & 0xFFF) as u16,
        ((w0 >> 8) & 0xFFF) as u16,
        (((w0 & 0xFF) << 4) | ((w1 >> 28) & 0xF)) as u16,
        ((w1 & 0x0FFF_0000) >> 16) as u16,
        ((w1 & 0xFFF0) >> 4) as u16,
        (((w1 & 0xF) << 8) | ((w2 & 0xFF00_0000) >> 24)) as u16,
        ((w2 >> 12) & 0xFFF) as u16,
        (w2 & 0xFFF) as u16,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(steps: &[Step]) -> Vec<&UsbCommand> {
        steps
            .iter()
            .filter_map(|s| match s {
                Step::Command(c) => Some(c),
                Step::ClearHalt => None,
            })
            .collect()
    }

    #[test]
    fn receiver_mode_and_frequency_are_encoded_as_libairspy_does() {
        let c = receiver_mode(true);
        assert_eq!((c.request, c.value, c.index), (1, 1, 0));
        assert_eq!(c.data, CommandData::Out(vec![]));
        assert_eq!(receiver_mode(false).value, 0);
        // 225.648 MHz = 0x0D7B_E100... little-endian.
        let c = set_freq(225_648_000).unwrap();
        assert_eq!((c.request, c.value, c.index), (13, 0, 0));
        assert_eq!(
            c.data,
            CommandData::Out(225_648_000u32.to_le_bytes().to_vec())
        );
        assert_eq!(
            c.data,
            CommandData::Out(vec![0x80, 0x1D, 0x73, 0x0D]),
            "225 648 000 = 0x0D731D80"
        );
    }

    #[test]
    fn frequencies_outside_the_tuner_are_refused() {
        assert!(set_freq(23_999_999).is_err());
        assert!(set_freq(1_800_000_001).is_err());
        assert!(set_freq(24_000_000).is_ok());
        assert!(set_freq(1_800_000_000).is_ok());
    }

    #[test]
    fn the_set_calls_are_one_byte_in_transfers_with_the_value_in_windex() {
        for (c, req, idx) in [
            (set_sample_rate(1), 12, 1),
            (set_packing(true), 26, 1),
            (set_packing(false), 26, 0),
            (set_lna_agc(false), 17, 0),
            (set_mixer_agc(true), 18, 1),
        ] {
            assert_eq!(c.request, req);
            assert_eq!((c.value, c.index), (0, idx));
            assert_eq!(c.data, CommandData::In(1));
        }
        let g = set_gains(AirspyGains {
            lna: 14,
            mixer: 7,
            vga: 3,
        });
        assert_eq!((g[0].request, g[0].index), (14, 14));
        assert_eq!((g[1].request, g[1].index), (15, 7));
        assert_eq!((g[2].request, g[2].index), (16, 3));
    }

    #[test]
    fn gains_are_clamped_before_they_are_sent() {
        let g = AirspyGains {
            lna: 99,
            mixer: 99,
            vga: 99,
        };
        assert_eq!(
            g.clamped(),
            AirspyGains {
                lna: 14,
                mixer: 15,
                vga: 15
            }
        );
        let c = set_gains(g);
        assert_eq!([c[0].index, c[1].index, c[2].index], [14, 15, 15]);
    }

    #[test]
    fn the_sample_rate_list_is_read_in_two_steps() {
        let c = read_sample_rate_count();
        assert_eq!((c.request, c.value, c.index), (25, 0, 0));
        assert_eq!(c.data, CommandData::In(4));
        let n = parse_sample_rate_count(&2u32.to_le_bytes()).unwrap();
        assert_eq!(n, 2);
        let c = read_sample_rates(n);
        assert_eq!((c.request, c.value, c.index), (25, 0, 2));
        assert_eq!(c.data, CommandData::In(8));
        let mut reply = Vec::new();
        reply.extend(10_000_000u32.to_le_bytes());
        reply.extend(2_500_000u32.to_le_bytes());
        let rates = parse_sample_rates(&reply, n).unwrap();
        assert_eq!(rates, [10_000_000, 2_500_000]);
        assert_eq!(iq_rate_index(&rates).unwrap(), 0);
        assert_eq!(iq_rate_index(&[2_500_000, 10_000_000]).unwrap(), 1);
        assert!(iq_rate_index(&[2_500_000]).is_err());
        assert!(parse_sample_rates(&reply[..7], n).is_err());
        assert!(parse_sample_rate_count(&[1, 0]).is_err());
        assert!(parse_sample_rate_count(&0u32.to_le_bytes()).is_err());
        assert!(parse_sample_rate_count(&u32::MAX.to_le_bytes()).is_err());
    }

    #[test]
    fn the_version_string_stops_at_the_nul() {
        assert_eq!(
            parse_version(b"AirSpy NOS v1.0.0-rc10-6-g4008e3c\0\0\0"),
            "AirSpy NOS v1.0.0-rc10-6-g4008e3c"
        );
        assert_eq!(parse_version(b"abc"), "abc");
        assert_eq!(board_name(0), "AIRSPY");
    }

    #[test]
    fn presets_follow_the_tables_from_the_most_gain_down() {
        // n = 21 is index 0: the most gain.
        assert_eq!(
            AirspyGains::linearity(21),
            AirspyGains {
                lna: 14,
                mixer: 12,
                vga: 13
            }
        );
        assert_eq!(
            AirspyGains::sensitivity(21),
            AirspyGains {
                lna: 14,
                mixer: 12,
                vga: 13
            }
        );
        // n = 0 is index 21: the least.
        assert_eq!(
            AirspyGains::linearity(0),
            AirspyGains {
                lna: 0,
                mixer: 0,
                vga: 4
            }
        );
        assert_eq!(
            AirspyGains::sensitivity(0),
            AirspyGains {
                lna: 0,
                mixer: 0,
                vga: 4
            }
        );
        // n = 15 is index 6.
        assert_eq!(
            AirspyGains::sensitivity(15),
            AirspyGains {
                lna: 14,
                mixer: 10,
                vga: 7
            }
        );
        assert_eq!(
            AirspyGains::linearity(15),
            AirspyGains {
                lna: 9,
                mixer: 6,
                vga: 11
            }
        );
        // Beyond 21 is 21.
        assert_eq!(AirspyGains::linearity(200), AirspyGains::linearity(21));
        assert_eq!(AirspyGains::default(), AirspyGains::sensitivity(15));
    }

    #[test]
    fn every_preset_is_within_the_hardware_ranges() {
        for n in 0..=PRESET_MAX {
            for g in [AirspyGains::linearity(n), AirspyGains::sensitivity(n)] {
                assert_eq!(g, g.clamped(), "preset {n}");
            }
        }
    }

    #[test]
    fn presets_parse_from_a_command_line() {
        assert_eq!(
            AirspyGains::parse_preset("sensitivity:15").unwrap(),
            AirspyGains::sensitivity(15)
        );
        assert_eq!(
            AirspyGains::parse_preset("Linearity:3").unwrap(),
            AirspyGains::linearity(3)
        );
        for bad in [
            "sensitivity",
            "sensitivity:22",
            "gain:3",
            "linearity:x",
            ":",
        ] {
            assert!(AirspyGains::parse_preset(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_start_sequence_follows_libairspys_order() {
        let s = AirspySettings::default();
        let steps = start_sequence(&s, 0).unwrap();
        let cmds = commands(&steps);
        let reqs: Vec<u8> = cmds.iter().map(|c| c.request).collect();
        assert_eq!(reqs, [21, 12, 26, 13, 17, 18, 14, 15, 16, 1, 1]);
        // The last two steps: halt cleared, receiver on.
        assert_eq!(steps[steps.len() - 2], Step::ClearHalt);
        assert_eq!(steps.last(), Some(&Step::Command(receiver_mode(true))));
        // The mode is switched off before it is switched on.
        assert_eq!(cmds[cmds.len() - 2].value, 0);
        // An out-of-range frequency fails before anything is sent.
        let bad = AirspySettings { center_hz: 5, ..s };
        assert!(start_sequence(&bad, 0).is_err());
    }

    #[test]
    fn no_command_sequence_ever_switches_the_bias_tee_on() {
        let c = bias_tee_off();
        assert_eq!(c.request, request::GPIO_WRITE);
        assert_eq!(c.value, 0, "wValue is the pin level: 0");
        assert_eq!(c.index, 45, "port 1, pin 13");
        assert_eq!(c.data, CommandData::Out(vec![]));
        for packed in [false, true] {
            for gains in [
                AirspyGains::default(),
                AirspyGains::linearity(21),
                AirspyGains {
                    lna: 255,
                    mixer: 255,
                    vga: 255,
                },
            ] {
                let s = AirspySettings {
                    packed,
                    gains,
                    ..Default::default()
                };
                for step in start_sequence(&s, 1).unwrap() {
                    if let Step::Command(c) = step
                        && c.request == request::GPIO_WRITE
                    {
                        assert_eq!(c.value, 0);
                        assert_eq!(c.index, 45);
                    }
                }
            }
        }
        // And the device layer never builds a GPIO write of its own: it only
        // sends what this module makes.
        let device = include_str!("airspy.rs");
        assert!(!device.contains("GPIO_WRITE"));
        assert!(!device.contains("gpio"));
    }

    /// The inverse of `unpack8`, to test it against: 8 × 12 bits into three
    /// words, first sample in the top bits of the first word.
    fn pack8(o: [u16; 8]) -> [u8; 12] {
        let o = o.map(u32::from);
        let w0 = (o[0] << 20) | (o[1] << 8) | (o[2] >> 4);
        let w1 = ((o[2] & 0xF) << 28) | (o[3] << 16) | (o[4] << 4) | (o[5] >> 8);
        let w2 = ((o[5] & 0xFF) << 24) | (o[6] << 12) | o[7];
        let mut b = [0u8; 12];
        b[0..4].copy_from_slice(&w0.to_le_bytes());
        b[4..8].copy_from_slice(&w1.to_le_bytes());
        b[8..12].copy_from_slice(&w2.to_le_bytes());
        b
    }

    #[test]
    fn unpacking_is_bit_exact() {
        // One bit at a time: each of the 96 input bits lands in exactly one
        // sample, at the place libairspy's formulas put it.
        for bit in 0..96 {
            let mut g = [0u8; 12];
            g[bit / 8] |= 1 << (bit % 8);
            let o = unpack8(&g);
            assert_eq!(
                o.iter().map(|v| v.count_ones()).sum::<u32>(),
                1,
                "bit {bit}"
            );
        }
        // Hand-worked: w0 = 0xABC_DEF_12 , w1 = 0x3_456_789_A , w2 = 0xBC_DEF_012.
        let (w0, w1, w2) = (0xABCD_EF12u32, 0x3456_789Au32, 0xBCDE_F012u32);
        let mut g = [0u8; 12];
        g[0..4].copy_from_slice(&w0.to_le_bytes());
        g[4..8].copy_from_slice(&w1.to_le_bytes());
        g[8..12].copy_from_slice(&w2.to_le_bytes());
        assert_eq!(
            unpack8(&g),
            [0xABC, 0xDEF, 0x123, 0x456, 0x789, 0xABC, 0xDEF, 0x012]
        );
        // Round trip over a spread of values, including the extremes.
        let mut x = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut o = [0u16; 8];
            for v in &mut o {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                *v = (x & 0xFFF) as u16;
            }
            assert_eq!(unpack8(&pack8(o)), o);
        }
        for o in [
            [0u16; 8],
            [0xFFF; 8],
            [2048; 8],
            [0, 0xFFF, 0, 0xFFF, 0, 0xFFF, 0, 0xFFF],
        ] {
            assert_eq!(unpack8(&pack8(o)), o);
        }
    }

    #[test]
    fn transfers_hold_whole_groups_of_samples() {
        assert_eq!(TRANSFER_BYTES % 16, 0);
        assert_eq!(TRANSFER_BYTES_PACKED % 12, 0);
        // 8 samples every 12 bytes, 4 pairs: 98 304 samples.
        assert_eq!(TRANSFER_BYTES_PACKED / 12 * 8, 98_304);
    }
}
