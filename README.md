<!-- made by CasualArclamp -->
# DecDAB

If you've found this worth your time, please consider [donating via PayPal](https://www.paypal.me/Arclamp).

A **DAB / DAB+ receiver** written in Rust, for the **HackRF One**, the **Airspy R2**
and **RTL-SDR** dongles, or recorded IQ. It decodes DAB+ (HE-AAC v1/v2) and
classic DAB (MP2) audio, the text and pictures that ride along with it, and
packet-mode data services, in a command-line tool and a desktop GUI.

The signal processing follows the ETSI specs (EN 300 401, TS 102 563,
EN 301 234, TS 101 756) and ISO/IEC 11172-3, cited clause by clause in the
source. CPU efficiency is the main design goal: a whole ensemble plus one
service decodes in about 7–8 % of one core from a 4.096 MS/s recording.

## Download

Portable Windows builds (`decdab-gui.exe`, `decdab.exe`) are on the
[Releases](https://github.com/CasualArclamp/DecDAB/releases) page. No installer:
download and run.

## Features

- **OFDM front end:** null-symbol detection, fractional and whole-carrier
  frequency correction, timing from the channel impulse response, residual CFO
  tracking per symbol.
- **FIC:** labels, services, components, sub-channels (EEP and UEP), programme
  type, broadcast time and user applications.
- **Audio:** DAB+ superframes with Reed–Solomon correction and HE-AAC v2
  decoding (Fraunhofer FDK-AAC); MP2 through a Rust Layer II decoder, with
  optional CRC checking and concealment of damaged frames.
- **Programme data:** Dynamic Label (DLS) with DL Plus, MOT slideshow through
  X-PAD (not yet seen on air).
- **Packet mode:** SPI service logos and programme guide (EPG XML), the MOT
  directory.
- **Desktop GUI (egui):** tuning dial, Band III spectrum and waterfall with the
  block raster, the ensemble card with a MER bar against what the code rate
  needs, the service list, channel sounder, programme guide, and an SFN view of
  the transmitters (TII) with their delays.
- **Recording:** raw IQ captures, the playing service as MP3, a once-a-second
  reception log (CSV), window screenshots (F12).

## Radios

| Radio | Notes |
|---|---|
| HackRF One | Band III is often weak: try `--amp --lna 40 --vga 20`. |
| Airspy R2 | Pure-Rust driver; libairspy-style gain presets (`--airspy-preset sensitivity:15`). |
| RTL-SDR | RTL2832U with an R820T/R820T2/R828D tuner, 2.048 MS/s. Less tested than the others. |
| ADALM-PLUTO / Pluto+ | Pure-Rust driver over USB or the network (`--pluto-uri auto\|usb\|ip:ADDRESS`, `--pluto-gain 50\|agc`). Receive only. Not yet tried on a real Pluto. |
| Fobos SDR | Needs RigExpert's `fobos_sdr.dll` (agile firmware) or `fobos.dll` (stock) beside the exe; runs at 8 MS/s. Not yet tried on the radio. |
| SDRplay RSP | Needs the SDRplay API installed (sdrplay.com). `--sdrplay-gain LNASTATE,IFGR`. Not yet tried on the radio. |
| USRP | Through UHD (`uhd.dll`, Windows; `--uhd-args`, `--uhd-gain`). Not yet tried on the radio. |
| DecSAT server | A radio on another machine, behind [DecSAT](https://github.com/CasualArclamp/DecSAT)'s `decsat-server`: `--radio remote --remote HOST`. The link is not encrypted. |
| Network IQ | Raw IQ from SDR++'s IQ Exporter, GNU Radio or rtl_tcp: `--radio net --net tcp://127.0.0.1:7777`. Set the sender to the rate asked for (4.096 MS/s) and tune it at its end. |

The Fobos, SDRplay and USRP drivers load their maker's library when the radio
is started, so DecDAB runs without them.

## Using it

```bash
decdab blocks                         # the Band III plan
decdab scan --amp                     # which blocks carry an ensemble
decdab info 12B --amp                 # an ensemble's services, live
decdab capture 12B --amp --seconds 10 # record IQ to captures/
decdab info captures/FILE.cs8         # the same from a recording
decdab decode captures/FILE.cs8 -s "Service name" -o out.wav   # one service to WAV, with CPU per stage
decdab-gui                            # the GUI
```

Pick the radio with `--radio hackrf|airspy|rtlsdr|pluto|fobos|sdrplay|uhd|remote|net`. Recordings are named by
their tuning (`12b_225.648MHz_4.096Msps_amp.cs8`) and the tools read the centre
frequency and sample rate from the name.

## Building

Rust stable (rustup) and a C compiler for FDK-AAC:

```bash
git clone --recursive https://github.com/CasualArclamp/DecDAB
cd DecDAB
cargo build --release
cargo run --release -p decdab-gui
```

If you cloned without `--recursive`, run `git submodule update --init` first.
The first build takes a minute while FDK-AAC compiles. On Windows,
`scripts\build-portable.ps1` builds the same portable exes as the releases.

## Licence

GPL-3.0-or-later. FDK-AAC is used under its own licence; code borrowed from
other projects is credited where it is used. Design notes and progress are in
[docs/DESIGN.md](docs/DESIGN.md) and [docs/STATUS.md](docs/STATUS.md).
