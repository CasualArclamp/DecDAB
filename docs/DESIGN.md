<!-- made by CasualArclamp -->
# DecDAB — Design

A Rust **DAB / DAB+** receiver (and matching modulator) for a **HackRF One**,
an **Airspy R2** or an **RTL-SDR**, with IQ-file replay. The sibling of DecDVB-T: the same crate shapes, the same
GUI look (spectrum and waterfall with the tuning, the signal card and
services on the right, scopes along the bottom, the statistics bar).

Status date: 2026-10-24 (kickoff). This document holds the scope and the
milestone plan and is the source of truth for both. The decisions come from
the kickoff Q&A with Rory.

---

## 0. The main requirement: CPU efficiency

As for DecDVB-T, **CPU efficiency comes first**. DAB is light (1.5 MHz, DQPSK,
a K = 7 code at a few hundred kbit/s), so the target is a few percent of one
core for a whole receiver with audio:

- The HackRF runs at 4.096 MS/s (twice DAB's 2.048 MHz clock); one halfband
  stage (47 taps, 12 multiplies an output) brings it to 2.048 MS/s. No
  resampler: the sample clock's error only drifts the timing, which the
  phase reference follows by whole samples.
- The OFDM demodulator does 77 FFTs of 2048 points a frame (96 ms) and
  nothing else per sample. Differential demodulation needs no channel
  estimate.
- **Only the selected sub-channel goes through the Viterbi decoder**; the
  FIC (32 kbit/s) is decoded always.
- Measure every stage (`decdab decode` prints CPU per stage).

## 1. Kickoff decisions

| Topic | Decision |
|---|---|
| Relation to DecDVB-T | **Fresh workspace** in `..\DecDAB`; crates copied from DecDVB-T (io, dsp, core) and DecDRM (FDK-AAC binding, data applications, charset) and renamed. |
| Audio | **DAB+ (HE-AAC v2) through FDK-AAC** (DecDRM's vendored, patched build); **MPEG Layer II in our own Rust** (`decdab-codecs::mp2`). |
| First release | Receiver, **modulator + channel models**, **DLS + MOT slideshow**, **Band III scan + ensemble list**, **EPG (SPI), TPEG, Journaline**. |
| Decoding | **One service played, all listed**: FIC for the whole ensemble, only the chosen sub-channel through FEC. |
| Standards | ETSI EN 300 401 (DAB), TS 102 563 (DAB+), EN 301 234 (MOT), TS 101 499 (slideshow), TS 102 818 / 102 371 (SPI), TS 102 979 (Journaline), TS 101 756 (tables), ISO/IEC 11172-3 and 13818-3 (Layer II). Mode I only (Band III). |
| Region | **UK Band III** (5A–13F). The roof aerial is UHF-filtered, so Band III is weak: `--amp --lna 40 --vga 20`. |
| Front end | **HackRF One** (live), **Airspy R2** (live; added 2026-10-27, §6), **RTL-SDR** with an R820T/R828D (live; added 2026-10-27, §7), IQ files. |
| Repo | Local git; GitHub **CasualArclamp/DecDAB, public** (since 2026-10-24). |
| Licence | GPL-3.0-or-later, `made by CasualArclamp` headers. FDK-AAC is under its own Fraunhofer licence (vendored, as in DecDRM). |

## 2. Crates

| Crate | What |
|---|---|
| `decdab-core` | Samples, errors, the Band III block plan, CRC-16, DAB charsets. |
| `decdab-io` | HackRF, Airspy R2 and RTL-SDR (pure-Rust drivers), the Airspy's real-to-IQ converter, IQ files, WAV IQ. From DecDVB-T. |
| `decdab-dsp` | FIR, DDC, resampler, AGC, DC. From DecDVB-T. |
| `decdab-ofdm` | Mode I parameters, phase reference, frequency interleaver, the front-end decimator, the demodulator (null/PRS sync, CFO, DQPSK → soft bits). |
| `decdab-fec` | Rate-1/4 K = 7 code, puncturing (FIC, EEP-A/B, UEP Table 31), block Viterbi, energy dispersal, time interleaving, RS(120,110). |
| `decdab-fic` | FIB decoding and the FIG parser → `Ensemble` (services, components, sub-channels, labels, PTy, user apps, time). |
| `decdab-msc` | A sub-channel's CIFs → logical frames; DAB+ superframes (Fire code, RS, AUs, PAD). |
| `decdab-codecs` | `AacDecoder` (FDK-AAC, raw AUs with an ASC from the superframe header), `Mp2Decoder` (with an optional header-CRC / ScF-CRC check and concealment, off by default). |
| `decdab-fdk-sys` | FDK-AAC build and FFI (from DecDRM). |
| `apps/decdab-cli` | `blocks`, `scan`, `capture`, `info`, `decode`. |

Planned: `decdab-pad` (X-PAD/F-PAD, DLS, MOT data groups), `decdab-data`
(MOT, slideshow, SPI, Journaline, TPEG; from DecDRM's `decdrm-data`, packet
mode rewritten for DAB), `decdab-mod` (ensemble generator and channel
models), `decdab-engine`, `decdab-audio` (player), `apps/decdab-gui`.

## 3. Application shape (GUI)

As DecDVB-T: frequency dial and source in the top bar; spectrum and
waterfall with the block raster and a VFO; on the right the **ensemble
card** (label, EId, MER, CFO, FIB rate, time) and the **service list**
(label, DAB/DAB+, bit rate, protection, PTy; click to play) with the
playing service's **DLS text and slideshow**; the bottom panel holds
**scopes** (DQPSK constellation, impulse response, SNR per carrier, null
symbol/TII) | **channel** | **history** | none; the statistics bar under it.
Band III scan lists ensembles and services.

## 4. Milestones

- **M0** — workspace, copied crates, docs, git. *(done 2026-10-24)*
- **M1** — HackRF front end, block plan, `scan`, `capture`; first captures
  of the London ensembles. *(done 2026-10-24: 11A, 11B, 11D, 12A, 12B, 12C)*
- **M2** — DAB modulator (FIC + MSC, DAB+ and MP2 sub-channels) and channel
  models.
- **M3** — OFDM receiver and FIC → ensemble list. *(done 2026-10-24; every
  capture locks at once, FIBs 100 %)*
- **M4** — MSC, Viterbi, DAB+ and MP2 audio to WAV. *(done 2026-10-24; live
  from the HackRF)*
- **M5** — PAD: DLS and MOT slideshow; then SPI/EPG, Journaline, TPEG.
- **M6** — engine, player and GUI in DecDVB-T's look.
- **M7** — TII (transmitter identification) and a transmitter map, release,
  GitHub CI.

## 5. Constraints found

- Band III arrives weak on this aerial (it carries a UHF band-pass filter):
  with the amp on, the ensembles are 7–13 dB over the gaps between blocks
  and the DQPSK MER is 11–16 dB, enough for error-free decoding.
- The HackRF's frequency error is small here (3–20 Hz at 220 MHz).
- No reference implementation is cloned; the tables come from the specs
  and are checked by construction (every UEP/EEP profile's bit counts) and
  by the real signal (FIB CRCs, RS, Fire code, AU CRCs).

## 6. Airspy R2 front end (2026-10-27)

A second live radio beside the HackRF, in `decdab-io` behind the `airspy`
cargo feature (on by default in the CLI and GUI). **Pure Rust over USB** on
`nusb`, as the HackRF driver: no libairspy, no libusb, no DLLs; on Windows the
Airspy must be on the WinUSB driver (Zadig). The protocol is libairspy's
(`airspy.c`, BSD-3-Clause; credited in the modules).

| File | What |
|---|---|
| `airspy_proto.rs` | The USB requests as pure functions (`UsbCommand`: request, value, index, data), the start sequence, the gain tables (libairspy's `linearity`/`sensitivity` presets 0–21), the 12-bit unpacker. No USB: fully unit-tested. |
| `airspy_dsp.rs` | `IqConverter`: the R2's 20 MS/s **real** stream to complex baseband at 2.048, 4.096 or 8.192 MS/s; `RawStream`: the buffer hand-over with retune/skip. No USB: tested with tones and a fake reader. |
| `airspy.rs` | The device: reader thread (8 bulk transfers of 256 KiB in flight, a bounded queue of 64, dropped buffers counted, buffers recycled), `AirspyControl` (retune, gains, overflows), `AirspySource: IqSource`, `airspy_info()`. |

**Bias tee: never on.** Sent off at open and at every start; the only
`GPIO_WRITE` is built by `bias_tee_off()` (no parameter), and a test checks the
sequences and that `airspy.rs` does not mention the request.

**Converter** (the R820T's IF is 5 MHz = fs/4 of the 20 MS/s ADC):
1. load: 12-bit offset-binary to f32, a block-wise DC estimate off, the
   fs/4 translation's signs on; even samples are I, odd ones Q;
2. libairspy's 31-tap half-band (16 non-zero taps, symmetric, centre 0.5),
   decimating by two: it touches only the even samples (an 8-multiply
   pre-added FIR for I) and Q is the centre tap alone (the odd samples
   delayed by 8, ×0.5), so the Q path is free;
3. zero to two further half-band decimators (Kaiser, 60 dB, 15 taps at
   10 MS/s and 23 at 5 MS/s, flat to ±0.8 MHz): one at 4.096 MS/s, two at
   2.048, none at 8.192;
4. a polyphase rational resampler (ratio 512/625 whichever the input rate:
   10, 5 or 2.5 MS/s to 8.192, 4.096 or 2.048) with a Kaiser prototype, 65 dB,
   8, 8 and 24 taps a phase.

Everything runs in 4096-sample chunks in preallocated buffers (no per-block
allocation), written as element-wise loops over contiguous slices so LLVM
vectorises them. **Amplitude:** a full-scale ADC tone comes out |IQ| = 1.0,
the same as a full-scale cs8 complex tone from the HackRF (`OUTPUT_GAIN` = 2
makes up the half a real tone loses); the engine's clip figure (0.99) means the
same on both. **Quality (tests):** flat to within 0.1 dB across ±0.768 MHz at
all three rates; aliasing into the ensemble from anywhere else in the
converter's band is below −57 dBc (target −50).

**Orientation.** Whether the band comes out mirrored is decided in one place,
`airspy_dsp::MIRROR_SPECTRUM` (false = libairspy's sign pattern, which assumes
the R820T presents RF `centre + δ` at IF `5 MHz − δ`). The test derives its
expectation from the constant. Not checkable without a radio.

**Gains.** AGC off for the LNA and mixer; the three stages are indices
(LNA 0–14, mixer 0–15, VGA 0–15). Default `sensitivity:15` (LNA 14, mixer 10,
VGA 7), a guess for a weak Band III aerial: **untested**.

**CLI:** `--radio hackrf|airspy` (default hackrf) wherever live gains were
accepted; `--airspy-preset sensitivity:N|linearity:N`, `--airspy-lna/mixer/vga`
(over the preset), `--airspy-packed`; `decdab airspy` prints the board,
firmware and sample rates; `capture --radio airspy` writes `.cs16`
(`..._airspy.cs16`). **GUI:** the Source combo has "Airspy R2"; its control row
shows LNA/mixer/VGA and preset menus instead of the HackRF's LNA/VGA/amp; the
SDR window shows the same and hides the HackRF-only filter, frequency
correction and optimiser; `--radio` on the command line; the radio, and the
Airspy's gains, are remembered (keys `radio`, `air_lna`, `air_mixer`,
`air_vga`; old preference files still load). The GUI's recorder still writes
8-bit `cs8` from the engine's stream.

**Not done:** the Airspy Mini/HF+ (other sample rates, HF+ is a different
protocol), 12-bit packing in the GUI (CLI only), frequency correction (the
R2's TCXO is good to ~0.5 ppm), a `decdab-io` hardware-in-the-loop test other
than the ignored `live_airspy`.

## 7. RTL-SDR front end (2026-10-27)

A third live radio: RTL2832U dongles with a Rafael Micro **R820T / R820T2 /
R828D** tuner (the RTL-SDR Blog V3 and V4 and most dongles of the last
decade), in `decdab-io` behind the `rtlsdr` feature (default-on in the CLI and
GUI). Pure Rust over `nusb` like the Airspy: no librtlsdr, no libusb, no DLLs;
on Windows the dongle must be on the WinUSB driver (Zadig), on Linux the DVB-T
kernel driver is detached at open. The register sequences, tables and formulas
are librtlsdr's (`librtlsdr.c`, `tuner_r82xx.c`, GPL-2.0-or-later) and, for the
V4's input switching, the RTL-SDR Blog fork's; credited in the modules.

| File | What |
|---|---|
| `rtlsdr_proto.rs` | The RTL2832U (baseband bring-up, FIR, resampler, IF, sample-clock correction, I2C repeater, FIFO reset) and the R82xx (init, IF-filter calibration, DVB-T system settings, RF mux, PLL, IF filter by bandwidth, gain, standby) as a driver over an `RtlBus` trait (vendor control out / in, sleep). The pure sums (`Pll`, `if_filter`, `resampler_ratio`, `gain_indices`, …) stand alone. No USB: run against a mock dongle in the tests. `U8ToIq`: the unsigned bytes to IQ, scaled as cu8. |
| `rtlsdr.rs` | The device: `RtlBus` on `nusb`, the shared bulk reader (8 transfers of 64 KiB in flight, a queue of 64), `RtlSdrControl` (retune, gain, ppm, overflows), `RtlSdrSource: IqSource`, `rtlsdr_info()`. |
| `usb.rs` | What the Airspy and RTL-SDR drivers share: `block_on_timeout` and the `BulkReader` thread (moved out of `airspy.rs`). `airspy_dsp::RawStream` takes any `RawToIq` converter now. |

**Rate:** 2.048 MS/s only, which the RTL2832U makes exactly from 28.8 MHz (ratio
0x0384_0000). 4.096 is beyond what the dongle streams (3.2 MS/s at most), so the
CLI and GUI run it at 2.048 whatever rate is asked for. The R82xx's IF filter
follows the rate as in librtlsdr: 2.05 MHz wide, the channel at a 1.625 MHz IF,
the spectrum inverted back in the RTL2832U.

**Gain:** the tuner's AGC, or librtlsdr's 29 manual steps (0–49.6 dB, the LNA
and mixer raised in turn, the VGA fixed at 16.3 dB). Default 40.2 dB manual, a
guess for a weak Band III aerial. **Frequency correction:** ppm, applied as
librtlsdr does (the RTL2832U's sample-clock register and the tuner's PLL
reference). **Bias tee:** GPIO 0 is driven low at open and never high (a test
checks every GPO write).

**RTL-SDR Blog V4** (found by its USB strings, `RTLSDRBlog` / `Blog V4`): its
R828D runs from 28.8 MHz, not 16, and its inputs are switched by band (VHF for
Band III) with its notch filters opened inside the FM band and Band III.

**CLI:** `--radio rtlsdr`, `--rtl-gain DB|agc`, `--rtl-ppm`; `decdab rtlsdr`
prints the dongle and its tuner; `capture --radio rtlsdr` writes the dongle's
own bytes as `.cu8` (`..._2.048Msps_rtlsdr.cu8`). **GUI:** "RTL-SDR" in the
Source combo; the rate shows 2.048 MS/s fixed; AGC and a gain step in the
control row and the SDR window, with the frequency correction; `--radio
rtlsdr`; the radio and its gain are remembered (keys `radio`, `rtl_agc`,
`rtl_gain_db10`).

**Not done:** other tuners (E4000, FC0012/13, FC2580: refused at open with a
message), direct sampling, rates other than 2.048, the RTL2832U's digital AGC,
the bias tee, offset tuning. **Untested on a dongle:** written without one.
