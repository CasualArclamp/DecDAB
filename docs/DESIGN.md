<!-- made by CasualArclamp -->
# DecDAB — Design

A Rust **DAB / DAB+** receiver (and matching modulator) for a **HackRF One**,
with IQ-file replay. The sibling of DecDVB-T: the same crate shapes, the same
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
| Front end | **HackRF One** (live), IQ files. |
| Repo | Local git; GitHub **CasualArclamp/DecDAB, public** (since 2026-10-24). |
| Licence | GPL-3.0-or-later, `made by CasualArclamp` headers. FDK-AAC is under its own Fraunhofer licence (vendored, as in DecDRM). |

## 2. Crates

| Crate | What |
|---|---|
| `decdab-core` | Samples, errors, the Band III block plan, CRC-16, DAB charsets. |
| `decdab-io` | HackRF (pure-Rust driver), IQ files, WAV IQ. From DecDVB-T. |
| `decdab-dsp` | FIR, DDC, resampler, AGC, DC. From DecDVB-T. |
| `decdab-ofdm` | Mode I parameters, phase reference, frequency interleaver, the front-end decimator, the demodulator (null/PRS sync, CFO, DQPSK → soft bits). |
| `decdab-fec` | Rate-1/4 K = 7 code, puncturing (FIC, EEP-A/B, UEP Table 31), block Viterbi, energy dispersal, time interleaving, RS(120,110). |
| `decdab-fic` | FIB decoding and the FIG parser → `Ensemble` (services, components, sub-channels, labels, PTy, user apps, time). |
| `decdab-msc` | A sub-channel's CIFs → logical frames; DAB+ superframes (Fire code, RS, AUs, PAD). |
| `decdab-codecs` | `AacDecoder` (FDK-AAC, raw AUs with an ASC from the superframe header), `Mp2Decoder`. |
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
