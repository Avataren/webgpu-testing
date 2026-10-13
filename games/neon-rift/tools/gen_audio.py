"""Synthesize the Neon Rift sound effects as 16-bit mono WAV files.

Usage: python3 gen_audio.py <output-dir>
"""
import sys
import wave
import numpy as np

SR = 44100
rng = np.random.default_rng(11)


def t_axis(dur):
    return np.arange(int(SR * dur)) / SR


def env(n, attack=0.003, decay=6.0):
    t = np.arange(n) / SR
    a = np.clip(t / attack, 0, 1)
    return a * np.exp(-decay * t / (n / SR))


def sweep(f0, f1, dur, shape="sine"):
    t = t_axis(dur)
    f = f0 * (f1 / f0) ** (t / dur)           # exponential glide
    phase = 2 * np.pi * np.cumsum(f) / SR
    if shape == "square":
        return np.sign(np.sin(phase))
    if shape == "saw":
        return 2 * ((phase / (2 * np.pi)) % 1.0) - 1
    if shape == "tri":
        return 2 / np.pi * np.arcsin(np.sin(phase))
    return np.sin(phase)


def note(freq, dur, decay=7.0, shape="tri"):
    w = sweep(freq, freq, dur, shape) + 0.35 * sweep(freq * 2, freq * 2, dur, "sine")
    return w * env(len(w), decay=decay)


def lowpass(x, k):
    out = np.copy(x)
    for i in range(1, len(x)):
        out[i] = out[i - 1] + k * (x[i] - out[i - 1])
    return out


def noise(dur):
    return rng.normal(0, 1, int(SR * dur))


def save(name, x, gain=0.8):
    x = x / (np.max(np.abs(x)) + 1e-9) * gain
    pcm = (np.clip(x, -1, 1) * 32767).astype(np.int16)
    with wave.open(f"{OUT}/{name}.wav", "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(SR)
        f.writeframes(pcm.tobytes())


OUT = sys.argv[1]

# shoot: short zappy downward chirp (played many times a second, so keep it tiny)
w = sweep(1500, 420, 0.07, "square") * 0.6 + 0.4 * sweep(750, 210, 0.07, "tri")
save("shoot", w * env(len(w), attack=0.001, decay=7.0), gain=0.6)

# hit: enemy takes damage but survives, a dull metallic tick
w = sweep(340, 220, 0.06, "square") + 0.5 * noise(0.06) * np.exp(-np.arange(int(SR * 0.06)) / SR * 90)
save("hit", w * env(len(w), attack=0.001, decay=8.0), gain=0.6)

# kill: noise burst with a falling thump
dur = 0.32
n = lowpass(noise(dur), 0.35)
thump = sweep(220, 55, dur, "sine")
w = n * 0.7 + thump
save("kill", w * env(len(w), attack=0.001, decay=5.0), gain=0.85)

# big kill: longer, deeper boom for the brute
dur = 0.7
n = lowpass(noise(dur), 0.18)
thump = sweep(150, 38, dur, "sine")
w = n * 0.8 + 1.2 * thump
save("boom", w * env(len(w), attack=0.001, decay=3.5), gain=0.95)

# player hit: harsh descending buzz
dur = 0.45
w = sweep(520, 70, dur, "saw") + 0.6 * noise(dur) * 0.3
w *= 0.8 + 0.2 * np.sin(2 * np.pi * 30 * t_axis(dur))
save("player_hit", w * env(len(w), attack=0.002, decay=3.0), gain=0.9)

# pickup: bright rising two-note blip
parts = [note(880, 0.07, decay=5.0, shape="tri"), note(1320, 0.14, decay=6.0, shape="tri")]
save("pickup", np.concatenate(parts))

# dash: airy whoosh
dur = 0.22
n = lowpass(noise(dur), 0.25)
swell = np.sin(np.pi * np.clip(t_axis(dur) / dur, 0, 1)) ** 1.5
w = n * swell + 0.4 * sweep(300, 900, dur, "sine") * swell
save("dash", w, gain=0.6)

# wave start: rising power chord
dur = 0.55
w = sum(sweep(f, f * 1.5, dur, "saw") * 0.3 for f in (110, 165, 220))
w = lowpass(w, 0.2) * env(len(w), attack=0.02, decay=2.5)
save("wave", w, gain=0.8)

# wave clear: little arpeggio
parts = [note(f, 0.1, decay=5.0) for f in (523, 659, 784, 1047)]
save("clear", np.concatenate(parts + [note(1047, 0.3, decay=4.0)]))

# nova: when the ship is hit and releases a shockwave
dur = 0.6
n = lowpass(noise(dur), 0.3)
w = sweep(90, 700, dur, "sine") * 0.8 + n * 0.5
save("nova", w * env(len(w), attack=0.01, decay=3.0), gain=0.9)

# start: arcade "coin" jingle
parts = [note(f, 0.09, decay=6.0, shape="square") for f in (392, 523, 659, 784)]
save("start", np.concatenate(parts + [note(1047, 0.35, decay=3.5, shape="square")]), gain=0.7)

# game over: slow descending line
parts = [note(f, 0.28, decay=4.0, shape="tri") for f in (392, 330, 262, 196, 147)]
save("gameover", np.concatenate(parts))
print("ok")
