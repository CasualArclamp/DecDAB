# The v0.2 feature video: what is new (the guide, station logos, the SFN
# panel, the icon), cut on the beat of beat.wav like promo.py, from GUI clips:
#   clip_guide.mp4  12B recorded 330 s, Radio 4 playing, --bottom guide, from 317 s
#   clip_sfn.mp4    11A, --bottom sfn, from 10 s
# both cropped to 1400x705 (the status bar off). Run from out/promo2 with
# icon.png (apps/decdab-gui/assets/icon-256.png) beside it.
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from promo import BAR, ACCENT, BLUE, POS, card_seg, clip_seg, ff, frames_for, text_file, zoom, FPS, W, H  # noqa: E402


def icon_seg(idx, bars, bg_clip, bg_t, title, sub):
    """The new icon, big, over a blurred still, with a line under it."""
    n = frames_for(bars)
    dur = n / FPS
    still = f"seg/bg{idx}.png"
    ff(["-ss", str(bg_t), "-i", f"clip_{bg_clip}.mp4", "-frames:v", "1",
        "-vf", f"scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},"
        "boxblur=18:2,eq=brightness=-0.4:saturation=1.2", still])
    tt = text_file(f"seg/icon{idx}a.txt", title)
    ts = text_file(f"seg/icon{idx}b.txt", sub)
    # The icon grows in over the first 0.3 s.
    size = "if(lt(t,0.3),300+120*t/0.3,420)"
    graph = (
        f"[0:v]{zoom(1.0, 1.05, dur)}[bg];"
        f"[1:v]format=rgba,scale=w='{size}':h='{size}':eval=frame:flags=lanczos[ic];"
        f"[bg][ic]overlay=x=(W-w)/2:y=330-h/2:eval=frame,"
        f"drawtext=fontfile=impact.ttf:expansion=none:textfile={tt}:fontsize=110:fontcolor=white:"
        f"x=(w-text_w)/2:y=600:shadowcolor=black@0.7:shadowx=4:shadowy=4:alpha='min(1,t/0.3)',"
        f"drawtext=fontfile=bahnschrift.ttf:expansion=none:textfile={ts}:fontsize=46:fontcolor={BLUE}:"
        f"x=(w-text_w)/2:y=740:shadowcolor=black@0.7:shadowx=3:shadowy=3:alpha='if(lt(t,0.4),0,min(1,(t-0.4)/0.2))',"
        f"fade=in:st=0:d=0.2:color=white[v]"
    )
    ff(["-framerate", str(FPS), "-loop", "1", "-i", still, "-loop", "1", "-i", "icon.png",
        "-filter_complex", graph, "-map", "[v]", "-frames:v", str(n),
        "-c:v", "libx264", "-preset", "medium", "-crf", "17", "-pix_fmt", "yuv420p", f"seg/{idx:02d}.mp4"])


b = BAR
POS[0] = 0.0
# 1. Title (2 bars, over the riser).
card_seg(1, 2, "guide", 6, [
    ("DecDAB 0.2", "impact.ttf", 230, "white", 300, 0.9),
    ("WHAT'S NEW", "bahnschrift.ttf", 64, BLUE, 610, 1.9),
    ("guide · station logos · SFN · a new look", "bahnschrift.ttf", 42, ACCENT, 710, 2.8),
], flash=False)
# 2. The drop: the guide.
clip_seg(2, "guide", 0.5, 2, big="THE PROGRAMME GUIDE",
         small="schedules from the ensemble's own SPI service")
# 3. On now.
clip_seg(3, "guide", 3.5, 1, crop="480:270:222:428", big="WHAT'S ON NOW",
         small="The Archers Omnibus, 11:00-12:15", z=(1.0, 1.1))
# 4. Logos.
clip_seg(4, "guide", 6.0, 1, crop="460:259:0:430", big="STATION LOGOS",
         small="for every service, straight off air", z=(1.0, 1.03))
# 5. The SFN panel.
clip_seg(5, "sfn", 0.5, 2, big="SEE EVERY TRANSMITTER",
         small="the SFN's TII, decoded from the null symbol")
# 6. The table.
clip_seg(6, "sfn", 4.0, 1, crop="400:225:0:432", big="WHO, HOW LOUD, HOW FAR",
         small="11/01 arrives 33 µs late: 10 km farther", z=(1.0, 1.03))
# 7. The comb grid and the paths.
clip_seg(7, "sfn", 7.0, 1, crop="444:250:345:432", big="ON THE IMPULSE RESPONSE",
         small="each transmitter on its own echo", z=(1.0, 1.1))
# 8. The icon.
icon_seg(8, 2, "sfn", 6, "A NEW LOOK", "a DQPSK scope, in green")
# 9. Under the hood.
card_seg(9, 2, "guide", 10, [
    ("PACKET MODE", "impact.ttf", 150, "white", 250, 0.0),
    ("SPI · MOT · EPG", "impact.ttf", 150, ACCENT, 450, b / 2),
    ("TII · 24 COMBS × 8", "impact.ttf", 150, BLUE, 650, b),
])
# 10. The end card.
card_seg(10, 2, "sfn", 9, [
    ("DecDAB 0.2.0", "impact.ttf", 200, "white", 300, 0.0),
    ("GUIDE · LOGOS · SFN · OUT NOW", "bahnschrift.ttf", 52, BLUE, 590, 0.4),
    ("github.com/CasualArclamp/DecDAB", "bahnschrift.ttf", 40, ACCENT, 680, 0.8),
])

with open("seg/list.txt", "w") as f:
    for i in range(1, 11):
        f.write(f"file '{i:02d}.mp4'\n")
ff(["-f", "concat", "-safe", "0", "-i", "seg/list.txt", "-c", "copy", "seg/video.mp4"])
total = 16 * BAR
ff(["-i", "seg/video.mp4", "-i", "beat.wav", "-t", f"{total + 0.6:.3f}",
    "-filter_complex",
    f"[0:v]tpad=stop_mode=clone:stop_duration=1,fade=out:st={total - 0.2:.3f}:d=0.8[v];"
    f"[1:a]afade=out:st={total - 0.2:.3f}:d=0.8[a]",
    "-map", "[v]", "-map", "[a]", "-c:v", "libx264", "-preset", "medium", "-crf", "18",
    "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart",
    "DecDAB_0.2_features.mp4"])
print("DecDAB_0.2_features.mp4")
