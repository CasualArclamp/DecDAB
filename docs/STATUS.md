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

### Gaps

- FIG 0/13 user applications are not yet attached to audio services on D1
  (look at the order 0/2 and 0/13 arrive in).
- Ensemble label shows the EId until FIG 1/0 arrives (the CLI picks the
  service after 10 frames).
- No PAD, DLS, slideshow, data services, modulator, GUI yet.
- MP2's ScF-CRC is not checked; a damaged frame plays as it is.
