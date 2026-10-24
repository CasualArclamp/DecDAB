# The promo's music: 16 bars at 128 BPM in A minor, synthesized (no samples,
# nothing anyone else owns). Bars 1-2: hats and a riser; the drop on bar 3;
# a final hit and a tail at the end.
import wave
import numpy as np

SR = 48000
BPM = 128
BEAT = 60 / BPM
BAR = 4 * BEAT
BARS = 16
N = int(SR * BAR * BARS) + SR  # a second of tail
mix = np.zeros((N, 2))
rng = np.random.default_rng(7)


def put(sig, t, gain=1.0, pan=0.0):
    i = int(t * SR)
    if i >= N:
        return
    sig = sig[: N - i]
    l, r = np.cos((pan + 1) * np.pi / 4), np.sin((pan + 1) * np.pi / 4)
    mix[i : i + len(sig), 0] += sig * gain * l * 1.414
    mix[i : i + len(sig), 1] += sig * gain * r * 1.414


def kick():
    t = np.arange(int(0.45 * SR)) / SR
    f = 45 + 110 * np.exp(-t * 30)
    ph = 2 * np.pi * np.cumsum(f) / SR
    s = np.sin(ph) * np.exp(-t * 6.5)
    click = rng.standard_normal(len(t)) * np.exp(-t * 400) * 0.3
    return np.tanh(2.2 * (s + click)) * 0.9


def clap():
    t = np.arange(int(0.3 * SR)) / SR
    n = rng.standard_normal(len(t))
    n = np.convolve(n, [1, -0.6], "same")  # a brighter noise
    env = np.exp(-t * 22) + 0.5 * np.exp(-((t - 0.012) ** 2) / 1e-5)
    return n * env * 0.35


def hat(open_=False):
    d = 0.18 if open_ else 0.05
    t = np.arange(int(d * SR)) / SR
    n = rng.standard_normal(len(t))
    n = n - np.convolve(n, np.ones(4) / 4, "same")  # high pass
    return n * np.exp(-t * (18 if open_ else 80)) * 0.22


def bass(freq, dur):
    t = np.arange(int(dur * SR)) / SR
    saw = 2 * ((t * freq) % 1) - 1
    sub = np.sin(2 * np.pi * freq / 2 * t)
    s = 0.55 * saw + 0.6 * sub
    # A one-pole low pass whose cutoff falls through the note.
    out = np.empty_like(s)
    y = 0.0
    for i, x in enumerate(s):
        a = 0.08 + 0.25 * np.exp(-t[i] * 8)
        y += a * (x - y)
        out[i] = y
    env = np.minimum(1, t / 0.005) * np.exp(-t * 1.5)
    return np.tanh(1.8 * out * env) * 0.45


def pad_chord(freqs, dur):
    t = np.arange(int(dur * SR)) / SR
    s = sum(np.sin(2 * np.pi * f * t + 0.3 * np.sin(2 * np.pi * 0.3 * t)) for f in freqs)
    env = np.minimum(1, t / 0.3) * np.minimum(1, (dur - t) / 0.3)
    return s / len(freqs) * env * 0.12


def riser(dur):
    t = np.arange(int(dur * SR)) / SR
    n = rng.standard_normal(len(t))
    k = np.linspace(2, 40, len(t)).astype(int)
    out = np.array([n[max(0, i - k[i]) : i + 1].mean() if i else 0 for i in range(len(t))])
    out = n - out  # brighter as it climbs
    sweep = np.sin(2 * np.pi * np.cumsum(np.linspace(200, 1800, len(t))) / SR) * 0.15
    return (out * 0.25 + sweep) * (t / dur) ** 2


def impact():
    t = np.arange(int(2.0 * SR)) / SR
    boom = np.sin(2 * np.pi * (40 + 60 * np.exp(-t * 8)) * t) * np.exp(-t * 2.5)
    noise = rng.standard_normal(len(t)) * np.exp(-t * 4) * 0.4
    return np.tanh(1.5 * (boom + noise)) * 0.9


# A minor: Am, F, C, G (root notes for the bass, triads for the pad).
roots = [55.0, 43.65, 65.41, 49.0]
chords = [[220, 261.6, 329.6], [174.6, 220, 261.6], [261.6, 329.6, 392], [196, 246.9, 293.7]]

k, c = kick(), clap()
for bar in range(BARS):
    t0 = bar * BAR
    for b in range(4):
        tb = t0 + b * BEAT
        # Hats all the way: closed on the off-beats, open on the last of a bar.
        put(hat(open_=(b == 3)), tb + BEAT / 2, pan=0.3)
        if bar >= 2:
            put(hat(), tb + BEAT / 4, 0.5, pan=-0.3)
            put(hat(), tb + 3 * BEAT / 4, 0.5, pan=-0.3)
    if bar < 2 or bar == BARS - 1:
        continue
    for b in range(4):
        put(k, t0 + b * BEAT)
        if b in (1, 3):
            put(c, t0 + b * BEAT, pan=0.1)
    root = roots[bar % 4]
    # Off-beat bass, the classic pump.
    for b in range(4):
        put(bass(root, BEAT * 0.5), t0 + b * BEAT + BEAT / 2)
    put(pad_chord(chords[bar % 4], BAR), t0)

put(riser(2 * BAR), 0, 0.9)
put(impact(), 2 * BAR, 1.0)
put(impact(), (BARS - 1) * BAR, 1.0)
put(pad_chord(chords[0], 2.5), (BARS - 1) * BAR, 1.5)

# Duck everything but the kicks a little on each beat (side-chain feel).
duck = np.ones(N)
for bar in range(2, BARS - 1):
    for b in range(4):
        i = int((bar * BAR + b * BEAT) * SR)
        L = int(0.25 * SR)
        duck[i : i + L] = np.minimum(duck[i : i + L], 0.55 + 0.45 * np.linspace(0, 1, L))
mix *= duck[:, None] ** 0.5
mix /= np.abs(mix).max() / 0.89
mix = np.tanh(mix * 1.2) / np.tanh(1.2)

with wave.open("beat.wav", "wb") as w:
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((mix * 32767).astype("<i2").tobytes())
print(f"beat.wav: {N / SR:.2f} s, bar {BAR:.4f} s")
