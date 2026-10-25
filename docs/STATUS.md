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

### Gaps

- FIG 0/13 is attached only when 0/2 came first (fine in practice; the
  commercial ensembles send no 0/13 at all).
- Ensemble label shows the EId until FIG 1/0 arrives (the CLI picks the
  service after 10 frames).
- No packet-mode data services (SPI/EPG, Journaline, TPEG) or modulator
  yet; the slideshow has not been seen on air (no London service in the
  captures signals one).
- The GUI has no band scan window, history or channel panels, and uses
  DecDVB-T's icon.
- GitHub: `gh` is installed but not signed in.
- MP2's ScF-CRC is not checked; a damaged frame plays as it is.
