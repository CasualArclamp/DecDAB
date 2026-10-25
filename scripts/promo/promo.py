# The DecDAB promo: GUI clips recorded from the app decoding London captures,
# cut on the beat of beat.wav (128 BPM, a bar is 1.875 s), with captions.
import os
import shutil
import subprocess

BAR = 60 / 128 * 4
W, H, FPS = 1920, 1080, 30
os.makedirs("seg", exist_ok=True)
for f in ("impact.ttf", "bahnschrift.ttf"):
    if not os.path.exists(f):
        shutil.copy(os.path.join(r"C:\Windows\Fonts", f), f)

ACCENT = "0x6ED282"  # the app's "playing" green
BLUE = "0x6EBEFF"


# Where the next segment starts, in bars: each segment is cut to the frame
# where its last beat falls, so the cuts stay on the beat.
POS = [0.0]


def frames_for(bars):
    a = round(POS[0] * BAR * FPS)
    POS[0] += bars
    return round(POS[0] * BAR * FPS) - a


def ff(args):
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", *args], check=True)


def text_file(name, text):
    with open(name, "w", encoding="utf-8") as f:
        f.write(text)
    return name


def caption(idx, big, small, dur):
    """Captions sliding in from the left: a green bar, a headline, a line."""
    tb = text_file(f"seg/cap{idx}a.txt", big)
    ts = text_file(f"seg/cap{idx}b.txt", small)
    slide = "if(lt(t,0.22),-900+(900+{x})*t/0.22,{x})"
    return (
        # A dark band behind them, so they read over a busy picture.
        f"drawbox=x=0:y={H - 300}:w={W}:h=300:color=black@0.62:t=fill,"
        f"drawbox=x='{slide.format(x=70)}':y={H - 250}:w=14:h=150:color={ACCENT}:t=fill,"
        f"drawtext=fontfile=impact.ttf:expansion=none:textfile={tb}:fontsize=96:fontcolor=white:"
        f"shadowcolor=black@0.8:shadowx=4:shadowy=4:x='{slide.format(x=110)}':y={H - 262},"
        f"drawtext=fontfile=bahnschrift.ttf:expansion=none:textfile={ts}:fontsize=40:fontcolor={BLUE}:"
        f"shadowcolor=black@0.8:shadowx=3:shadowy=3:x='{slide.format(x=114)}':y={H - 150}:"
        f"enable='gte(t,0.12)'"
    )


def zoom(z0, z1, dur, cx=0.5, cy=0.5):
    """A slow push-in from z0 to z1 about (cx, cy)."""
    n = int(dur * FPS)
    z = f"{z0}+({z1}-{z0})*on/{n}"
    return (
        f"zoompan=z='{z}':x='(iw-iw/zoom)*{cx}':y='(ih-ih/zoom)*{cy}':d=1:s={W}x{H}:fps={FPS}"
    )


def clip_seg(idx, clip, start, bars, crop=None, big="", small="", z=(1.0, 1.08)):
    n = frames_for(bars)
    dur = n / FPS
    chain = []
    if crop:
        chain.append(f"crop={crop}")
    # Full frames (1400x760) are letterboxed into 16:9 on black first.
    chain.append("scale=3840:-2:flags=lanczos")
    if not crop:
        chain.append("pad=3840:2160:(ow-iw)/2:(oh-ih)/2:color=0x0a0a0e")
    else:
        chain.append("scale=3840:2160:flags=lanczos,setsar=1")
    chain.append(zoom(z[0], z[1], dur))
    if big:
        chain.append(caption(idx, big, small, dur))
    chain.append("fade=in:st=0:d=0.15:color=white")
    ff(["-ss", str(start), "-i", f"clip_{clip}.mp4",
        "-vf", ",".join(chain), "-frames:v", str(n), "-an",
        "-c:v", "libx264", "-preset", "medium", "-crf", "17", "-pix_fmt", "yuv420p",
        f"seg/{idx:02d}.mp4"])


def card_seg(idx, bars, bg_clip, bg_t, lines, flash=True):
    """A title card over a blurred, darkened still of the app. `lines`:
    (text, font, size, colour, y, appears at s)."""
    n = frames_for(bars)
    dur = n / FPS
    still = f"seg/bg{idx}.png"
    ff(["-ss", str(bg_t), "-i", f"clip_{bg_clip}.mp4", "-frames:v", "1",
        "-vf", f"scale={W}:{H}:force_original_aspect_ratio=increase,crop={W}:{H},"
        "boxblur=18:2,eq=brightness=-0.32:saturation=1.3", still])
    chain = [zoom(1.0, 1.06, dur)]
    for j, (txt, font, size, col, y, at) in enumerate(lines):
        tf = text_file(f"seg/card{idx}_{j}.txt", txt)
        # Each line fades and rises in at its time.
        alpha = f"if(lt(t,{at}),0,min(1,(t-{at})/0.2))"
        ypos = f"{y}+if(lt(t,{at}+0.2),30*(1-(t-{at})/0.2),0)"
        chain.append(
            f"drawtext=fontfile={font}:expansion=none:textfile={tf}:fontsize={size}:fontcolor={col}:"
            f"alpha='{alpha}':x=(w-text_w)/2:y='{ypos}':shadowcolor=black@0.7:shadowx=4:shadowy=4"
        )
    if flash:
        chain.append("fade=in:st=0:d=0.2:color=white")
    ff(["-framerate", str(FPS), "-loop", "1", "-i", still, "-vf", ",".join(chain),
        "-frames:v", str(n), "-c:v", "libx264", "-preset", "medium", "-crf", "17",
        "-pix_fmt", "yuv420p", f"seg/{idx:02d}.mp4"])



def main():
    b = BAR
    segs = []
    # 1. Title over the riser (2 bars).
    card_seg(1, 2, "d1", 8, [
        ("DecDAB", "impact.ttf", 260, "white", 330, 0.9),
        ("DAB · DAB+ · LIVE FROM A HACKRF", "bahnschrift.ttf", 56, BLUE, 640, 1.9),
        ("written in Rust", "bahnschrift.ttf", 40, ACCENT, 730, 2.8),
    ], flash=False)
    # 2. The drop: the whole app on D1 National.
    clip_seg(2, "d1", 1.0, 2, big="LOCKS IN ONE FRAME", small="D1 National · 31 services · every FIB good")
    # 3. The multiplex strip on the waterfall.
    clip_seg(3, "d1", 5.0, 1, crop="490:276:230:150", big="EVERY SERVICE, EVERY CU",
             small="the playing service lit across the ensemble", z=(1.0, 1.12))
    # 4. The scopes.
    clip_seg(4, "d1", 8.0, 1, crop="440:248:0:455", big="DQPSK, LIVE", small="FIC and MSC scopes", z=(1.0, 1.1))
    # 5. BBC National.
    clip_seg(5, "bbc", 1.0, 2, big="MP2 IN PURE RUST", small="BBC National DAB · Radio 2 · 48 kHz stereo")
    # 6. The card's margin.
    clip_seg(6, "bbc", 6.0, 1, crop="495:278:905:60", big="KNOW YOUR MARGIN",
             small="MER against what the code needs", z=(1.0, 1.1))
    # 7. The impulse response: the SFN's transmitters.
    clip_seg(7, "bbc", 9.0, 1, crop="410:231:985:469", big="SEE THE SFN",
             small="every transmitter's echo, in microseconds", z=(1.0, 1.12))
    # 8. SDL, HE-AAC v2 and its dynamic label.
    clip_seg(8, "sdl", 2.0, 2, big="HE-AAC v2 · DLS · DL PLUS", small="SDL National · Absolute 80s")
    # 9. The numbers.
    card_seg(9, 2, "bbc", 10, [
        ("6 ENSEMBLES", "impact.ttf", 150, "white", 250, 0.0),
        ("130+ SERVICES", "impact.ttf", 150, ACCENT, 450, b / 2),
        ("~8 % OF ONE CORE", "impact.ttf", 150, BLUE, 650, b),
    ])
    # 10. The end card.
    card_seg(10, 2, "sdl", 6, [
        ("DecDAB", "impact.ttf", 230, "white", 300, 0.0),
        ("DAB+ · MP2 · DLS · SCOPES · HACKRF ONE", "bahnschrift.ttf", 48, BLUE, 600, 0.4),
        ("Rust · GPL-3.0", "bahnschrift.ttf", 40, ACCENT, 690, 0.8),
    ])

    with open("seg/list.txt", "w") as f:
        for i in range(1, 11):
            f.write(f"file '{i:02d}.mp4'\n")
    ff(["-f", "concat", "-safe", "0", "-i", "seg/list.txt", "-c", "copy", "seg/video.mp4"])
    total = 16 * BAR
    ff(["-i", "seg/video.mp4", "-i", "beat.wav", "-t", f"{total + 0.6:.3f}",
        "-filter_complex",
        # The picture holds its last frame through the music's tail, then fades.
        f"[0:v]tpad=stop_mode=clone:stop_duration=1,fade=out:st={total - 0.2:.3f}:d=0.8[v];"
        f"[1:a]afade=out:st={total - 0.2:.3f}:d=0.8[a]",
        "-map", "[v]", "-map", "[a]", "-c:v", "libx264", "-preset", "medium", "-crf", "18",
        "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart",
        "DecDAB_promo.mp4"])
    print("DecDAB_promo.mp4")


if __name__ == "__main__":
    main()
