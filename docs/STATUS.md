<!-- made by CasualArclamp -->
# DecDAB — Status

## 2026-10-28 — 0.2.3: selected-service MP3, session log, snapshots

Ported from a contributor's patches against 0.2.0 (OM5NA, issue #2); their
Airspy half was dropped, as 0.2.1 already has a driver. The GUI gains:

- **● MP3** records only the playing service as 192 kbit/s CBR MP3 under
  `captures/` (block, frequency, service name, SId and UTC in the name). The
  encoder (`rusty_mp3`, pure Rust) runs on its own thread, so the receiver
  thread only queues PCM; a backlog drops blocks rather than stalling.
  A service change, retune or new ensemble ends the file. Checked end to end
  with `decdab-engine --example mp3_probe` (ffprobe reads the result).
- **● Log** writes one CSV row a second (lock, MER, FIBs, level, drops).
- **📷 / F12** saves a PNG of the window to `captures/`.

Not tried on a live radio.

## 2026-10-24 — kickoff to live audio (M0, M1, M3, M4)

**Front end.** `decdab scan --amp` found six ensembles from the London roof:
11A (SDL National), 11B, 11D (D1 National), 12A, 12B (BBC National DAB),
12C; 5D and 9B faintly. Captures of each (10 s, and 60 s of 11D, 12B and 11A)
are in `captures/` at 4.096 MS/s; 12B also at 2.048 MS/s.

**OFDM.** Null symbol by energy, fractional CFO from the 76 guard intervals,
whole carriers from the phase reference (differential correlation), timing
from the impulse response's first path, residual CFO from the DQPSK
decisions (fourth power) per symbol and fed back. All six captures lock on
the first frame and never lose it; MER 11–16 dB.

**FIC.** All 12 FIBs a frame pass their CRC on every capture (one bad FIB in
7400). The FIG parser gives labels, services, components, sub-channels
(EEP and UEP), PTy, the time (0/10) and user apps (0/13).

**Audio.** DAB+: superframes sync, RS corrects the odd byte, every AU's CRC
holds; FDK-AAC decodes HE-AAC v2 with parametric stereo (32 kHz). MP2: the
Rust Layer II decoder plays BBC Radio 3, 4 and 6 Music at 48 kHz. Live
from the HackRF: Absolute Radio (11D) and 6 Music (12B), 15 s each, nothing
lost.

**CPU** (`decdab decode`, % of one core, 4.096 MS/s input): front end
(halfband) 2.3, OFDM 3.3, FIC 0.3, sub-channel 0.3–1.2 (32–160 kbit/s),
audio 0.2–0.4. Whole file run ~7.5 % including reading the file.

**PAD.** `decdab-data` (DecDRM's data crate: MOT, slideshow, SPI,
Journaline, data groups) gained `pad`: F-PAD/X-PAD with contents
indicators, DLS with DL Plus, MOT data groups via X-PAD. DLS decodes on
every service tried, DAB+ and MP2 (Absolute, Classic FM, Heart, Capital,
Smooth, LBC, BBC Radio 1/2/4), with no bad CRC. In a DAB MP2 frame the
ScF-CRC sits between X-PAD and F-PAD; it is 4 bytes with the 27/30-band
allocation tables (LBC's 24 kHz too) and 2 with the 8/12-band ones.
No London service in these captures signals a slideshow (FIG 0/13 is sent
only by 12B and 12C, for data services), so MOT via PAD waits for the
modulator.

**Engine and GUI** (DecDVB-T's layout): dial and controls, the Band III
band view and waterfall with the block raster, the DAB card (ensemble label,
MER bar with the margin against the code's required MER and its tick, ECC
with the code rate beside it, EId, services, FIBs, carrier offset, time,
capacity), the playing service (DLS, DL Plus tags, slideshow, codec and
frame counts, volume), the service list, the scopes (FIC and MSC DQPSK
points, gain and SNR per carrier, impulse response with echoes) and the
statistics bar. Live on 11D from the HackRF: locked, Absolute Radio
playing, whole app about 20 % of a core at 15 fps (OFDM 7 %, audio 1 %).

**Required MER** (`--example threshold`, noise added to captures): EEP 3-A
is error free from 10.5 dB of the receiver's MER (first lost AUs at
10.1–10.3 dB on 11A and 11D); the FIC loses its first FIBs at 9.8–9.9 dB.
11D sits at 11.6 dB: a margin of about 1 dB on this aerial. Other rates are
estimated (card.rs).

**Channel sounder** (2026-10-25), DecDVB-T's channel panel: a fading
waterfall (gain at every other carrier, a row a frame), the delay–Doppler
map and the Doppler spectrum by carrier. DAB has no pilots, so the channel
is estimated decision-directed: from each frame's phase reference, each
symbol's cells decided coherently against the last estimate smoothed over
five carriers, on the constellation that symbol uses (axes and diagonals
alternate); rows every 12 ms (Doppler ±37.5 Hz), delays from the first path,
the sample clock's drift taken out. On 11D: D1's SFN paths at 0 and 33 µs,
a faint one near 130 µs, all static. Deciding every symbol on the diagonals
had made frame-rate lines at ±10.4 Hz (`--example sound_probe` showed the
13–17° jumps at each frame's start).

**Packet mode, SPI: logos and the guide** (2026-10-25). No London audio
service sends MOT in its X-PAD (only DLS: application types 0/2/3 on every
service of six captures); the pictures travel in packet-mode data services
instead (BBC Guide on 12B, Station Logos on 12C, both SPI). `dab_packet.rs`
reads EN 300 401 §5.3.2 packets out of the logical frames (CRC per packet,
24-byte grid, packets that straddle frames carried over) and builds data
groups per address; packet-FEC parity packets (address 1022, nine per 2256
bytes, no CRC) are skipped, which had looked like 16 % bad packets. The
engine decodes the ensemble's SPI service in the background (0.2 % CPU) and
keeps logos and schedules (`spi.rs`); the GUI shows logos in the service
list, beside the playing service and in the slideshow's place, and a Guide
panel. On 12B: 2301 packets, none bad, 97 data groups, 10 logos (BBC
`siC22401.png`-style names, 32×32 to 320×240) in 60 s; the MOT directory
lists 85 objects, 539 kB, a carousel of about 270 s, so the schedule files
(`w20261025dC224c0.EHB`) come later. A 330 s recording got the whole
carousel: 12,902 packets, none bad; 15 schedules (Radio 4: 50 programmes,
the clock change on 25 October carried in the times) and the SI file. Its
service ids are binary DAB content ids (`40 e1 ce15 c221`), which DRM's
table had read as text; they now decode as `e1.ce15.c221.0`. The Guide panel
marked The Archers Omnibus on now, as the station's DLS said.

**SFN: TII** (2026-10-25). `tii.rs`: the null symbol's carriers averaged
(~2 s), each comb's eight groups against the noise, a transmitter where four
groups stand clear (main id = rank among the 70 four-bit words, layout after
welle.io). Delays from the phase slope across the comb's carriers are only
known modulo N/48 (20.8 µs); each is placed on the averaged impulse
response's strongest candidate path, the strongest transmitter on its peak.
Here: 08/23 first, 11/01 and 11/02 33 µs later on 11A; 08/23 then
11/06 at +33 µs on 12C; 11/01 at +17 µs and 08/23 on 12B. The GUI's SFN panel
lists them (level, delay, extra km, path level) over the impulse response and
the comb × group grid.

**Album art on 11A** (2026-10-25). A 240 s recording of SDL National: none
of its 30 services sends a slideshow. Every X-PAD carries only the dynamic
label (application types 0, 2, 3: no 1, 12 or 13), and the FIC has no FIG
0/13 in a minute (7488 FIBs). A test now feeds a picture through the X-PAD
path (length indicator, MOT start and continuation subfields among DLS
fields) and gets it back whole, so the decoder would show one.

**Logos on 12C, and a cache** (2026-10-25). 12C's Station Logos service
is Arqiva's shared SPI feed for D1 National, London 1 and SDL National: 236
objects, 1.6 MB, randomly named (`10f4z`), so about 27 minutes a turn at 8
kbit/s; a 10-minute recording got D1's SI file but not London 1's, so no 12C
service could be matched (and 92 of 346 data groups were lost to packet
errors, which packet FEC would mend). The store now keeps every SI file (it
kept only the last) and matches a dotted id's SId field exactly. Logos
matched to a service are saved to `%APPDATA%\DecDAB\logos` (`C1C0_128x128.png`)
and drawn from there until the air brings them: after the 12C run, 11D showed
Absolute Radio's and Heart 70s/80s's logos without any SPI of its own.

**MP2 error concealment** (2026-10-26), a switch, off by default. The
plain MP2 services (Virgin Radio, Premier, Fix Radio; UEP 3) protect the
scale factors only medium and the sample bits least, and a damaged scale
factor makes a sub-band "boil" (loud splats, pumping). `Mp2Decoder::set_conceal`
turns on two checks the decoder never made: the header CRC-16 (EN 300 401
Annex B.2; covers the header's second half, bit allocation and SCFSI) and the
ScF-CRC (Annex B.3: a CRC-8 per group of sub-bands 0–3, 4–7, 8–15, 16–, over
the 3 MSBs of their scale factors, sent in the *previous* frame's tail in
reverse group order). Only a detected failure is acted on: a bad header CRC
repeats the last good frame's sub-band samples, fading out across the frame
(silence if several in a row), through the synthesis filter, and fades the
next good frame in over 4 ms (before any good frame it is an error: no rate to
trust); a bad ScF-CRC group gets the previous frame's scale factors for its
allocated sub-bands (zero if it had none before). Sample-bit errors are
undetectable and left alone, and a frame that passes is decoded bit for bit as
with the switch off (tested; so is switch-off against the pre-change decoder,
by hash). After lost sync (`push` skipped bytes) the next frame's ScF-CRC is
not believed. Counters on the decoder (`hdr_crc_fail`, `scf_crc_fail`,
`concealed`) feed `ServiceStats` and the GUI's service panel; the GUI has a
"repair MP2 errors" checkbox (pref `mp2_conceal`, sent as
`Command::Mp2Conceal`, live) and `decdab decode` a `--mp2-conceal` flag. CPU:
unchanged within noise (about 0.13 % of a core for a 24 s-a-second stream,
off or on). **Verified** on 812 real frames of Virgin Radio (mono, 27-band, 48
kHz, 80 kbit/s) for both CRCs, and on the two-frame vector in the tests;
**not** checked on air: stereo and joint stereo (scale-factor order for the CRC
follows the bitstream order), the two-CRC 8/12-band case (groups 0–3 and 4–7,
from Annex B.3), MPEG-2 LSF, and the sound of the concealment on a real weak
signal (no captures with errors were at hand).

## 2026-10-27 — Airspy R2 front end

**Built in the cloud, with no radio.** A second live front end beside the
HackRF, in pure Rust over USB (`nusb`, no libairspy / libusb / DLLs):
`decdab-io` `airspy_proto.rs` (the protocol as pure functions, the gain
tables, the 12-bit unpacker), `airspy_dsp.rs` (the real-to-IQ converter and
the buffer hand-over), `airspy.rs` (device, reader thread, control handle,
`airspy_info()`), behind the `airspy` feature (default-on in the CLI and GUI).
CLI: `--radio hackrf|airspy`, `--airspy-preset`, `--airspy-lna/-mixer/-vga`,
`--airspy-packed`, `decdab airspy`; every existing HackRF invocation parses and
behaves as before. GUI: "Airspy R2" in the Source combo, its own gain row and
SDR-window section, `--radio`, remembered (old prefs still load). Design in
DESIGN.md §6.

**Verified offline** (`cargo test`, `clippy -D warnings`): every USB command's
encoding (request, value, index, data), the start sequence's order, the rate-list
parser, gain clamps and the 22-entry presets, the packed-sample unpacker bit by
bit and by round trip, that no sequence sends the bias tee anything but 0 (and
that `airspy.rs` builds no `GPIO_WRITE`); libairspy's half-band against its
spec with `rustfft` (ripple 0.013 dB to 3.9 MHz; **58.7 dB** stop band from
6.1 MHz, not the 62 dB quoted); the converter with synthetic 20 MS/s tones
through the real code (the same bytes packed and unpacked; any split of the input
into pieces): frequency and amplitude right at ±1 MHz (±0.7 at 2.048 MS/s),
flat within 0.1 dB over ±0.768 MHz at 2.048, 4.096 and 8.192 MS/s, worst alias
into the ensemble **−57.7 dBc** over tones anywhere in the converter's band, a
DC offset gone; a fake reader for the retune path (queue drained, in-flight
buffers skipped, buffers handed back). A full-scale ADC tone gives |IQ| = 1.0,
like a full-scale cs8 tone from the HackRF.

**CPU** of the whole real-to-IQ chain (`cargo test --release -p decdab-io
bench_converter -- --ignored --nocapture`; best of 9 on a shared 2.1 GHz Xeon
VM, so a desktop should do better): see the table. Input is 20 MS/s of
unpacked 16-bit samples (40 MB/s).

| output | half-bands | ns / raw sample | ns / output sample | % of one core at 20 MS/s in |
|---|---|---|---|---|
| 4.096 MS/s | 1 (default) | 2.74 | 13.4 | 5.5 |
| 4.096 MS/s | 0 | 3.27 | 16.0 | 6.5 |
| 4.096 MS/s, packed input | 1 | 2.94 | 14.3 | 5.9 |
| 2.048 MS/s | 2 (default) | 3.47 | 33.9 | 6.9 |
| 2.048 MS/s | 1 | 4.49 | 43.8 | 9.0 |
| 8.192 MS/s | 0 (default) | 3.63 | 8.9 | 7.3 |

(20 MS/s of raw samples is 10 MS/s of IQ, libairspy's `iqconverter` rate;
the 1.2 ns/raw sample of that is the load and libairspy's half-band, the rest
is the later stages.)

The default is the cheapest of each: 1 half-band at 4.096 MS/s, 2 at 2.048, none
at 8.192. For comparison the HackRF path's whole front end is 2.3 % at
4.096 MS/s (above): the Airspy's costs more because it has twice the samples
and a resampler to bring 10 MS/s to 4.096, in return for 12 bits. The USB side
(a reader thread handing 256 KiB buffers over) was not measured.

### Needs a live check on the owner's machine (Airspy R2, WinUSB)

In this order; none could be tried in the cloud.

*Update 2026-10-27.* The same USB and driver design (nusb, the start sequence,
the 10 MS/s stream, the fs/4 orientation constant) was proven live on a real
R2 in the sibling DecDVB-T receiver: it opens, streams 10 MS/s with no dropped
buffers, the spectrum is the right way round and tuning is accurate. That
covers items 1 to 3 for the shared code, not DAB reception itself: nothing here
has been run against a DAB signal yet, so the list stays as it is. One fix came
out of it: the firmware version read returns a shorter reply than asked
(48 bytes of 127), which is accepted (libairspy takes what comes back).

1. **Spectral orientation.** The sign pattern is libairspy's and assumes the
   R820T puts RF `centre + δ` at IF `5 MHz − δ`. Tune a block, look at the
   waterfall: a station whose edge you know (a neighbouring block, an FM
   carrier through a known filter) or `decdab info`/`decode` working at all: a
   mirrored band does not lock. If it is mirrored, set
   `airspy_dsp::MIRROR_SPECTRUM = true` (the one constant; the test follows it).
2. **Tuned frequency accuracy.** On a known ensemble the receiver reports its
   carrier offset (the GUI card; `decdab info`): expect a few tens of Hz, as
   the HackRF's 3–20 Hz at 220 MHz. A large offset means the IF the firmware
   assumes is not 5 MHz in the sample stream.
3. **40 MB/s unpacked without dropped buffers.** `decdab capture 12B --radio
   airspy --seconds 60` prints the dropped buffers (and the GUI's status bar);
   zero is expected. If not, try `--airspy-packed` (30 MB/s), and a different
   USB port (not a hub).
4. **The gain preset for Band III.** Default `sensitivity:15` (LNA 14, mixer 10,
   VGA 7) is a guess from the HackRF needing its amp, LNA 40 and VGA 20 here.
   Try `--airspy-preset sensitivity:12..21` and `linearity:` ones; the capture
   prints rms and clipping, and the GUI shows the MER.
5. **Retune between ensembles.** In the GUI step the blocks (◀ ▶) and type a
   block; `decdab scan --radio airspy` walks the band. Each retune throws away
   the queued buffers and the 9 next (about 60 ms); check an ensemble locks
   afresh after each and that the old one's services are gone.
6. **A real DAB / DAB+ decode and its MER against the HackRF.** `decdab decode
   11D --radio airspy -s "Classic FM" -o out/x.wav` (a DAB+ one and an MP2 one)
   and the GUI card's MER on the same block, aerial and hour as a HackRF run.
   The Airspy's 12 bits and the converter's filtering should give the same or
   a better MER.
7. Also unchecked: `decdab airspy` prints the board id, firmware string and
   the sample rates (R2: 10 and 2.5 MS/s); unplugging mid-stream gives an
   error rather than a hang; closing the app stops the radio (its LED / current).

### Gaps

- FIG 0/13 is attached only when 0/2 came first (fine in practice; the
  commercial ensembles send no 0/13 at all).
- Ensemble label shows the EId until FIG 1/0 arrives (the CLI picks the
  service after 10 frames).
- Packet mode carries SPI only so far (Journaline, TPEG not wired); a
  schedule or logo takes up to one carousel (270 s on 12B, ~27 min on
  12C's shared feed) to arrive; packet FEC
  parity is skipped, not used. No modulator yet; the X-PAD slideshow has
  not been seen on air.
- TII names no transmitters (no database); a weak TII (the 130 µs path)
  is not identified.
- The GUI has no band scan window or history panel.
- MP2's header and ScF CRCs are checked only with the "repair MP2 errors" switch on (off by default); the 3 LSBs of a scale factor and the sample bits are unprotected, so damage there still plays as it is.

## 2026-10-27 — RTL-SDR front end

**Built without a dongle.** A third live front end: RTL2832U + R820T/R820T2/R828D
(RTL-SDR Blog V3/V4 and the like), in pure Rust over USB (`nusb`, no librtlsdr /
libusb / DLLs): `decdab-io` `rtlsdr_proto.rs` (both chips as a driver over a
bus trait, the PLL / filter / gain sums standing alone), `rtlsdr.rs` (the device
on `nusb`, the control handle, `rtlsdr_info()`), behind the `rtlsdr` feature
(default-on in the CLI and GUI). The Airspy's `block_on_timeout` and bulk reader
moved to a shared `usb.rs`, and `RawStream` takes any `RawToIq` converter; the
Airspy behaves as before (its tests pass unchanged). CLI: `--radio rtlsdr`,
`--rtl-gain DB|agc`, `--rtl-ppm`, `decdab rtlsdr`, captures as `.cu8`. GUI:
"RTL-SDR" in the Source combo, 2.048 MS/s fixed, AGC + gain step, ppm in the SDR
window, remembered. Design in DESIGN.md §7.

**Verified offline** (`cargo test`, `clippy -D warnings`, against a mock dongle
that keeps the tuner's registers as written): the resampler ratio (exactly
2.048 MS/s), the FIR's 12-bit packing, the IF filter for 2.048 (0x0b = 0xaf,
IF 1.625 MHz) and wider/narrower bandwidths, the PLL within 55 Hz of the wanted
LO at every 16 kHz over 174–240 MHz (×16 and ×8 dividers) and its register
encoding, every one of the 29 gain steps reached exactly, the IF and ppm
register values; the bring-up end to end on an R820T, a plain R828D and a V4
(tuner registers for 11D: PLL, RF filters, IF filter, calibration code, gain;
the demodulator's inversion, ratio and IF), retune and ppm moving the PLL, AGC,
standby and power-down, an unknown tuner refused, the bias-tee pin never driven
high. The GUI was run with no dongle (the control row and SDR window show the
RTL-SDR's controls; opening reports none found).

**Not verified:** anything on a real dongle. The register sequences are
written out from librtlsdr from memory of its source, not checked against a
USB trace; the first live test is
`cargo test --release -p decdab-io --features rtlsdr live_rtlsdr -- --ignored --nocapture`,
then `decdab info 11D --radio rtlsdr`.
