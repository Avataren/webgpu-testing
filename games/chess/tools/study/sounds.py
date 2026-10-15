"""Chess sounds for the study, synthesised from simple physical models.

    python3 games/chess/tools/study/sounds.py [out_dir]

- move: a felted wooden piece set down on the board: a short knock of decaying
  wood modes over the table's low body resonance, in a small furnished room.
- capture: piece clacks against piece, the winner lands, the loser is set aside.
- check: a soft, low wooden tone under the move.
- end: a warm two-note mantel-clock chime.
- clock: the chess clock's plunger being pressed.

Each sound has variants (<name>_1..N.wav); <name>.wav is the first, for players that
use one file. Seeds are fixed, so the output is reproducible.
"""

import os
import struct
import sys
import wave

import numpy as np

RATE = 48000
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "study", "content", "audio")


def t_axis(seconds):
    return np.arange(int(seconds * RATE)) / RATE


def onepole_lowpass(x, cutoff):
    a = np.exp(-2 * np.pi * cutoff / RATE)
    y = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[i] = acc
    return y


def bandpass_noise(rng, seconds, low, high):
    n = int(seconds * RATE)
    spec = np.fft.rfft(rng.standard_normal(n))
    f = np.fft.rfftfreq(n, 1 / RATE)
    spec *= ((f > low) & (f < high)) * 1.0
    out = np.fft.irfft(spec, n)
    return out / (np.abs(out).max() + 1e-9)


def knock(rng, seconds=0.35, base=600.0, hardness=0.5, body=170.0, body_amp=0.5, felt=0.6):
    """A wooden impact: a filtered click, inharmonic wood modes and a low body thump.
    hardness raises the click and high modes; felt damps them."""
    t = t_axis(seconds)
    out = np.zeros_like(t)
    # Free-free bar ratios, detuned a little each time.
    ratios = [1.0, 2.76, 5.40, 8.93, 13.34]
    for i, r in enumerate(ratios):
        f = base * r * rng.uniform(0.97, 1.03)
        if f > 16000:
            break
        tau = 0.045 / (1 + i * (1.2 + felt * 2.0))
        amp = (0.8 ** i) * (hardness + (1 - hardness) * 0.5 ** i)
        out += amp * np.sin(2 * np.pi * f * t + rng.uniform(0, 6.28)) * np.exp(-t / tau)
    # Contact click: a millisecond of band-passed noise.
    click = bandpass_noise(rng, seconds, 1500 + 2500 * hardness, 9000 + 5000 * hardness)
    out += click * np.exp(-t / (0.0007 + 0.0008 * (1 - hardness))) * (0.5 + hardness) * (1 - 0.6 * felt)
    # The board and table: a short low thump with a soft attack.
    thump = np.sin(2 * np.pi * body * rng.uniform(0.95, 1.05) * t) * np.exp(-t / 0.035)
    thump *= 1 - np.exp(-t / 0.002)
    out += body_amp * thump
    # A tiny attack ramp avoids a digital click at sample zero.
    out *= 1 - np.exp(-t / 0.0002)
    return out


def room(signal, rng, wet=0.12, decay=0.18):
    """Small furnished room: a few early reflections and a short, dark diffuse tail."""
    n = len(signal) + int(0.45 * RATE)
    out = np.zeros(n)
    out[:len(signal)] += signal
    for delay, gain in ((0.0043, 0.22), (0.0071, 0.16), (0.0118, 0.12), (0.0167, 0.08), (0.0231, 0.06)):
        d = int(delay * RATE)
        out[d:d + len(signal)] += signal * gain * rng.uniform(0.8, 1.1)
    tail_t = t_axis(0.45)
    ir = rng.standard_normal(len(tail_t)) * np.exp(-tail_t / decay)
    ir = onepole_lowpass(ir, 2500)
    ir /= np.sqrt((ir ** 2).sum()) + 1e-9
    reverb = np.fft.irfft(np.fft.rfft(signal, n) * np.fft.rfft(ir, n), n)
    out += reverb * wet
    return out


def place(dst, src, at):
    i = int(at * RATE)
    end = min(len(dst), i + len(src))
    dst[i:end] += src[:end - i]


def finish(x, peak=0.7, fade=0.02):
    x = x - np.mean(x)
    # Trim the inaudible end, fade out the last bit.
    level = np.abs(x)
    above = np.nonzero(level > level.max() * 10 ** (-60 / 20))[0]
    x = x[: above[-1] + 1 if len(above) else len(x)]
    n = min(len(x), int(fade * RATE))
    x[-n:] *= np.linspace(1, 0, n) ** 2
    return x / (np.abs(x).max() + 1e-9) * peak


def write(name, x):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name + ".wav")
    data = np.clip(x, -1, 1)
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(v * 32767)) for v in data))
    return path


def move(rng):
    k = knock(rng, base=rng.uniform(540, 660), hardness=rng.uniform(0.35, 0.5), body=rng.uniform(150, 185),
              body_amp=0.55, felt=0.65)
    # A faint second contact as the piece settles.
    settle = knock(rng, seconds=0.2, base=rng.uniform(600, 700), hardness=0.25, body=170, body_amp=0.3, felt=0.8)
    x = np.zeros(int(0.4 * RATE))
    place(x, k, 0.0)
    place(x, settle * 0.12, rng.uniform(0.018, 0.03))
    return finish(room(x, rng, wet=0.05, decay=0.1), peak=0.62)


def capture(rng):
    clack = knock(rng, seconds=0.25, base=rng.uniform(1050, 1250), hardness=0.9, body=320, body_amp=0.1, felt=0.0)
    land = knock(rng, base=rng.uniform(540, 640), hardness=0.45, body=rng.uniform(150, 180), body_amp=0.55, felt=0.6)
    aside = knock(rng, base=rng.uniform(430, 520), hardness=0.3, body=rng.uniform(120, 140), body_amp=0.45, felt=0.75)
    x = np.zeros(int(0.9 * RATE))
    place(x, clack * 0.75, 0.0)
    place(x, land, rng.uniform(0.06, 0.08))
    place(x, aside * 0.55, rng.uniform(0.34, 0.42))
    return finish(room(x, rng, wet=0.05, decay=0.1), peak=0.66)


def check(rng):
    """A soft, low wooden tone: a marimba-like bar note, muted."""
    t = t_axis(1.2)
    f = 196.0 * rng.uniform(0.995, 1.005)
    x = (np.sin(2 * np.pi * f * t) * np.exp(-t / 0.42)
         + 0.35 * np.sin(2 * np.pi * f * 3.93 * t) * np.exp(-t / 0.12)
         + 0.12 * np.sin(2 * np.pi * f * 9.2 * t) * np.exp(-t / 0.04))
    x *= 1 - np.exp(-t / 0.004)
    return finish(room(x, rng, wet=0.18, decay=0.25), peak=0.42, fade=0.08)


def bell(t, f, amp=1.0):
    """A small clock bell: inharmonic partials with their own decays and beating pairs."""
    parts = [(0.5, 2.6, 0.35), (1.0, 2.2, 1.0), (1.19, 1.6, 0.45), (1.5, 1.3, 0.35), (2.0, 0.9, 0.3),
             (2.51, 0.6, 0.18), (3.0, 0.45, 0.12)]
    x = np.zeros_like(t)
    for ratio, tau, a in parts:
        for detune in (-0.6, 0.6):
            x += a * 0.5 * np.sin(2 * np.pi * (f * ratio + detune) * t) * np.exp(-t / tau)
    strike = np.exp(-t / 0.004) * np.sin(2 * np.pi * f * 4.2 * t) * 0.3
    return (x + strike) * (1 - np.exp(-t / 0.002)) * amp


def end(rng):
    t = t_axis(4.5)
    x = np.zeros(int(5.0 * RATE))
    place(x, bell(t, 523.25), 0.0)        # C5
    place(x, bell(t, 392.0, 0.9), 0.62)   # G4
    return finish(room(x, rng, wet=0.2, decay=0.3), peak=0.5, fade=0.4)


def clock(rng):
    """The plunger: a sharp mechanical click, a tiny spring ring, a wooden case knock."""
    t = t_axis(0.25)
    click = bandpass_noise(rng, 0.25, 2500, 12000) * np.exp(-t / 0.0009)
    ring = np.sin(2 * np.pi * rng.uniform(3100, 3400) * t) * np.exp(-t / 0.03) * 0.2
    case = knock(rng, seconds=0.25, base=rng.uniform(820, 900), hardness=0.6, body=240, body_amp=0.3, felt=0.2)
    x = np.zeros(int(0.3 * RATE))
    place(x, click + ring, 0.0)
    place(x, case * 0.6, 0.004)
    place(x, bandpass_noise(rng, 0.05, 2000, 8000) * np.exp(-t_axis(0.05) / 0.0006) * 0.4, 0.045)
    return finish(room(x, rng, wet=0.08), peak=0.45)


VARIANTS = {"chess_move": (move, 6), "chess_capture": (capture, 4), "chess_check": (check, 2),
            "chess_end": (end, 1), "chess_clock": (clock, 4)}


def main():
    global OUT
    if len(sys.argv) > 1:
        OUT = sys.argv[1]
    for name, (make, count) in VARIANTS.items():
        for i in range(count):
            rng = np.random.default_rng(sum(map(ord, name)) * 31 + i)
            x = make(rng)
            write(f"{name}_{i + 1}", x)
            if i == 0:
                write(name, x)
        print("wrote", name, count)


if __name__ == "__main__":
    main()
