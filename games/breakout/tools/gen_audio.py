"""Synthesize the Breakout sound effects as 16-bit mono WAV files.

Usage: python3 gen_audio.py <output-dir>
"""
import sys
import wave
import numpy as np

SR = 44100
rng = np.random.default_rng(3)


def t_axis(dur):
    return np.arange(int(SR * dur)) / SR


def env(n, attack=0.004, decay=6.0):
    t = np.arange(n) / SR
    a = np.clip(t / attack, 0, 1)
    return a * np.exp(-decay * t / (n / SR))


def sweep(f0, f1, dur, shape="sine"):
    t = t_axis(dur)
    f = f0 * (f1 / f0) ** (t / dur)           # exponential glide
    phase = 2 * np.pi * np.cumsum(f) / SR
    if shape == "square":
        return np.sign(np.sin(phase))
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


def save(name, x, gain=0.8):
    x = x / (np.max(np.abs(x)) + 1e-9) * gain
    pcm = (np.clip(x, -1, 1) * 32767).astype(np.int16)
    with wave.open(f"{OUT}/{name}.wav", "wb") as f:
        f.setnchannels(1)
        f.setsampwidth(2)
        f.setframerate(SR)
        f.writeframes(pcm.tobytes())


OUT = sys.argv[1]

# launch: quick rising chirp
w = sweep(260, 820, 0.18, "tri")
save("launch", w * env(len(w), decay=4.0))

# paddle: bright blip with a tiny pitch drop
w = sweep(520, 380, 0.10, "tri") + 0.3 * sweep(1040, 760, 0.10, "sine")
save("paddle", w * env(len(w), decay=7.0))

# wall: dull thunk
w = lowpass(sweep(200, 110, 0.09, "square"), 0.25)
save("wall", w * env(len(w), decay=8.0), gain=0.7)

# bricks: one pitch per row, red (top) highest
for i, f in enumerate([1047, 932, 784, 659, 523, 440]):
    n = note(f, 0.14, decay=8.0)
    click = rng.normal(0, 1, len(n)) * np.exp(-np.arange(len(n)) / SR * 90) * 0.25
    save(f"brick{i}", n + click, gain=0.75)

# lose a ball: long falling wobble
w = sweep(440, 70, 0.6, "tri")
w *= 0.75 + 0.25 * np.sin(2 * np.pi * 14 * t_axis(0.6))
save("lose", w * env(len(w), decay=3.0))

# level clear: rising arpeggio
parts = [note(f, 0.13, decay=5.0) for f in (523, 659, 784, 1047)]
tail = note(1047, 0.35, decay=4.0)
save("clear", np.concatenate(parts + [tail]))

# game over: slow descending line
parts = [note(f, 0.26, decay=4.0, shape="tri") for f in (392, 330, 262, 196)]
save("gameover", np.concatenate(parts))

# intro: soft whoosh while the bricks fly in
dur = 0.8
n = rng.normal(0, 1, int(SR * dur))
n = lowpass(n, 0.12)
swell = np.sin(np.pi * np.clip(t_axis(dur) / dur, 0, 1)) ** 2
w = n * swell + 0.35 * sweep(180, 720, dur, "sine") * swell
save("intro", w, gain=0.55)
print("ok")
