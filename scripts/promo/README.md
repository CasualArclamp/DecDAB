# The promo video

A 30 s, 1920×1080 promo cut on the beat (16 bars at 128 BPM), from GUI clips
the app records of itself decoding captures. Run from a folder outside the repo
(the outputs are big), e.g. `out/promo`:

```bash
# 1. clips: the GUI records its own frames (12 s each, from 5 s in), status bar cropped off
for c in "d1 captures/11d_long_222.064MHz_4.096Msps_amp.cs8 Heart UK" ...; do …; done
decdab-gui CAPTURE --play SERVICE --fps 30 --record-video rec_d1.mp4 --record-from 5 --record-secs 12
ffmpeg -i rec_d1.mp4 -vf crop=1400:705:0:0 clip_d1.mp4      # likewise bbc (12B, Radio 2), sdl (11A, Absolute 80s)
# 2. the music (synthesized, nobody else's) and the edit
python -I beat.py
python -I promo.py      # DecDAB_promo.mp4
```

The crops in promo.py assume a 1400×860 window at 100 % scaling.

## The 0.2 feature video

`features.py` cuts a second video the same way (same beat, `promo.py`'s
helpers), from two clips: `clip_guide` (the 330 s 12B recording, Radio 4,
`--bottom guide --record-from 317`, when the schedules are in) and
`clip_sfn` (11A, `--bottom sfn --record-from 10`), each cropped to
1400×705, with `icon.png` (the app's `icon-256.png`) beside them. Run it
from `out/promo2`: `python -I ../../scripts/promo/features.py` writes
`DecDAB_0.2_features.mp4`.
