<!-- made by CasualArclamp -->
# DecDAB — Status

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
