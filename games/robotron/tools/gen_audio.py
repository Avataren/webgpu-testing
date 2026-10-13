"""Synthesize the Robotron sound effects and a music loop as 16-bit mono WAV files.

Usage: python3 gen_audio.py <output-dir>
"""
import sys
import wave
import numpy as np

SR = 44100
rng = np.random.default_rng(2084)


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


def highpass(x, k):
    return x - lowpass(x, k)


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

# shoot: tiny bright zap (played ~10 times a second, so keep it small)
w = sweep(2100, 700, 0.055, "square") * 0.5 + 0.5 * sweep(1050, 350, 0.055, "tri")
save("shoot", w * env(len(w), attack=0.001, decay=7.0), gain=0.5)

# grunt_die: crunchy noise burst with a falling thump
dur = 0.26
n = lowpass(noise(dur), 0.4)
w = n * 0.7 + sweep(240, 60, dur, "sine")
save("grunt_die", w * env(len(w), attack=0.001, decay=5.5), gain=0.8)

# electrode_die: electric crackle
dur = 0.34
n = highpass(noise(dur), 0.3)
w = n * (0.5 + 0.5 * np.sign(np.sin(2 * np.pi * 70 * t_axis(dur)))) + 0.5 * sweep(900, 120, dur, "square")
save("electrode_die", w * env(len(w), attack=0.001, decay=5.0), gain=0.8)

# hulk_hit: dull metallic clank (hulks cannot be killed, only pushed)
dur = 0.12
w = sweep(190, 120, dur, "sine") + 0.6 * highpass(noise(dur), 0.2) * np.exp(-t_axis(dur) * 40)
save("hulk_hit", w * env(len(w), attack=0.001, decay=7.0), gain=0.7)

# rescue: bright rising three-note chime
parts = [note(f, 0.07, decay=5.0) for f in (784, 988)] + [note(1319, 0.2, decay=5.0)]
save("rescue", np.concatenate(parts), gain=0.8)

# human_die: falling wail with vibrato
dur = 0.6
vib = 1 + 0.05 * np.sin(2 * np.pi * 9 * t_axis(dur))
base = sweep(900, 200, dur, "tri") * vib
w = base + 0.4 * sweep(1800, 400, dur, "sine")
save("human_die", w * env(len(w), attack=0.01, decay=2.8), gain=0.8)

# convert: spooky ring-modulated wobble (a brain turns a human into a prog)
dur = 0.55
carrier = sweep(300, 180, dur, "sine")
ring = np.sin(2 * np.pi * 55 * t_axis(dur))
w = carrier * ring + 0.4 * sweep(120, 700, dur, "saw") * np.sin(np.pi * t_axis(dur) / dur)
save("convert", w * env(len(w), attack=0.01, decay=2.5), gain=0.8)

# player_die: long falling buzz with noise
dur = 1.2
w = sweep(800, 40, dur, "saw") + 0.5 * lowpass(noise(dur), 0.15)
w *= 0.75 + 0.25 * np.sin(2 * np.pi * 22 * t_axis(dur))
save("player_die", w * env(len(w), attack=0.002, decay=2.2), gain=0.95)

# wave_start: rising warp
dur = 0.7
w = sum(sweep(f, f * 3.0, dur, "saw") * 0.3 for f in (110, 165, 220))
w = lowpass(w, 0.25) * np.sin(np.pi * np.clip(t_axis(dur) / dur, 0, 1)) ** 0.7
save("wave_start", w, gain=0.8)

# wave_clear: arpeggio
parts = [note(f, 0.09, decay=5.0) for f in (523, 659, 784, 1047, 1319)]
save("wave_clear", np.concatenate(parts + [note(1568, 0.35, decay=4.0)]))

# extra_life: bright fanfare
parts = [note(f, 0.08, decay=4.0, shape="square") for f in (659, 784, 988, 1319)]
save("extra_life", np.concatenate(parts + [note(1568, 0.4, decay=3.0, shape="square")]), gain=0.65)

# brain_die: mid boom
dur = 0.5
n = lowpass(noise(dur), 0.22)
w = n * 0.8 + 1.1 * sweep(200, 45, dur, "sine")
save("brain_die", w * env(len(w), attack=0.001, decay=4.0), gain=0.9)

# missile: whoosh with a rising whine
dur = 0.35
n = lowpass(noise(dur), 0.3)
sw = np.sin(np.pi * np.clip(t_axis(dur) / dur, 0, 1))
w = n * sw * 0.8 + 0.4 * sweep(300, 900, dur, "tri") * sw
save("missile", w, gain=0.6)

# spark: enforcer zap
dur = 0.14
w = sweep(1400, 300, dur, "square") * 0.6 + 0.3 * highpass(noise(dur), 0.3)
save("spark", w * env(len(w), attack=0.001, decay=5.0), gain=0.55)

# tank_shot: thump with a metallic edge
dur = 0.25
w = sweep(170, 55, dur, "sine") * 1.2 + 0.5 * highpass(noise(dur), 0.25) * np.exp(-t_axis(dur) * 25)
save("tank_shot", w * env(len(w), attack=0.001, decay=5.0), gain=0.85)

# bounce: shell ricochet blip
dur = 0.07
w = sweep(900, 1400, dur, "tri")
save("bounce", w * env(len(w), attack=0.001, decay=6.0), gain=0.5)

# spawn: quick warp pop as an enemy materializes
dur = 0.16
w = sweep(200, 1800, dur, "sine") * np.sin(np.pi * np.clip(t_axis(dur) / dur, 0, 1))
save("spawn", w, gain=0.4)

# big_boom: spheroid / quark death
dur = 0.8
n = lowpass(noise(dur), 0.16)
w = n * 0.9 + 1.2 * sweep(160, 32, dur, "sine")
save("big_boom", w * env(len(w), attack=0.001, decay=3.2), gain=0.95)

# start: arcade coin jingle
parts = [note(f, 0.09, decay=6.0, shape="square") for f in (392, 523, 659, 784)]
save("start", np.concatenate(parts + [note(1047, 0.35, decay=3.5, shape="square")]), gain=0.7)

# game over: slow descending line
parts = [note(f, 0.3, decay=4.0, shape="tri") for f in (392, 330, 262, 196, 147)]
save("gameover", np.concatenate(parts))

# music: a 16-beat loop at 128 bpm (kick, saw bass, hats, and a minor arpeggio)
bpm = 128
beat = 60.0 / bpm
n_beats = 16
total = int(SR * beat * n_beats)
mix = np.zeros(total)


def put(sig, at, gain=1.0):
    i = int(at * SR)
    j = min(total, i + len(sig))
    mix[i:j] += sig[: j - i] * gain


kick = sweep(150, 42, 0.22, "sine") * env(int(SR * 0.22), attack=0.001, decay=7.0)
hat = highpass(noise(0.05), 0.4) * env(int(SR * 0.05), attack=0.001, decay=6.0)
bass_notes = [55.0, 55.0, 65.41, 55.0, 73.42, 55.0, 65.41, 49.0]  # A1 A1 C2 A1 D2 A1 C2 G1
arp = [220.0, 261.63, 329.63, 392.0]                                # A3 C4 E4 G4
for b in range(n_beats):
    put(kick, b * beat, 0.9)
    put(hat, b * beat + beat / 2, 0.35)
    f = bass_notes[(b // 2) % len(bass_notes)]
    for h in range(2):
        d = beat / 2
        w = lowpass(sweep(f, f, d, "saw"), 0.18) * env(int(SR * d), attack=0.004, decay=4.0)
        put(w, b * beat + h * d, 0.55)
    for s in range(4):
        fa = arp[(b * 4 + s) % len(arp)] * (2 if (b // 4) % 2 else 1)
        d = beat / 4
        put(note(fa, d, decay=6.0, shape="square") * 0.5, b * beat + s * d, 0.18)
save("music", mix, gain=0.6)
print("ok")
