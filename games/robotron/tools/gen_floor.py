"""Floor textures for the arena plane (38 x 24 m, UV 0..1 across it).

    python3 tools/gen_floor.py content/textures

floor_base.png      dark panels, 2 m tiles with seams and a little per-tile variation
floor_glow.png      emissive grid: faint lines every metre, brighter ones every 4 m
wall_spill.png      white with alpha falling off from the top row: the border's light on the
                    floor (the script tints it with the border's colour)
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image

W_M, H_M = 38, 24          # arena size in metres
PPM = 64                   # pixels per metre
W, H = W_M * PPM, H_M * PPM

# pixel centres in metres, origin at the arena's centre
x = (np.arange(W) + 0.5) / PPM - W_M / 2
z = (np.arange(H) + 0.5) / PPM - H_M / 2
X, Z = np.meshgrid(x, z)


def line(coord, period, width):
    """1 on lines every `period` metres (width in metres, gaussian falloff), 0 between."""
    d = np.abs(coord - np.round(coord / period) * period)
    return np.exp(-(d / width) ** 2)


def srgb(lin):
    lin = np.clip(lin, 0, 1)
    return np.where(lin <= 0.0031308, lin * 12.92, 1.055 * lin ** (1 / 2.4) - 0.055)


def save(rgb_lin, path):
    img = (srgb(rgb_lin) * 255 + 0.5).astype(np.uint8)
    Image.fromarray(img, "RGB").save(path)
    print("wrote", path, img.shape)


def base():
    rng = np.random.default_rng(2084)
    tiles = rng.uniform(0.85, 1.15, size=(H_M // 2 + 1, W_M // 2 + 1))
    ti = np.floor((X + W_M / 2) / 2).astype(int)
    tj = np.floor((Z + H_M / 2) / 2).astype(int)
    v = tiles[tj, ti]
    seam = np.maximum(line(X, 2, 0.025), line(Z, 2, 0.025))
    bevel = np.maximum(line(X + 0.06, 2, 0.03), line(Z + 0.06, 2, 0.03)) * 0.35
    lum = 0.055 * v * (1 - 0.75 * seam) * (1 + bevel)
    tint = np.array([0.78, 0.86, 1.15])
    return lum[..., None] * tint


def glow():
    minor = np.maximum(line(X, 1, 0.012), line(Z, 1, 0.012))
    major = np.maximum(line(X, 4, 0.02), line(Z, 4, 0.02))
    major_halo = np.maximum(line(X, 4, 0.12), line(Z, 4, 0.12))
    # distance to the border, metres
    edge = np.minimum(W_M / 2 - np.abs(X), H_M / 2 - np.abs(Z))
    edge_glow = np.exp(-edge / 0.9)
    # a slow falloff from the middle so the centre reads a bit brighter
    r = np.sqrt((X / (W_M / 2)) ** 2 + (Z / (H_M / 2)) ** 2)
    centre = 0.65 + 0.35 * np.exp(-r * r * 1.5)

    cyan = np.array([0.15, 0.45, 1.0])
    violet = np.array([0.55, 0.2, 1.0])
    out = (minor[..., None] * 0.10 * cyan
           + major[..., None] * 0.45 * cyan
           + major_halo[..., None] * 0.05 * cyan) * centre[..., None]
    out += edge_glow[..., None] * 0.04 * violet
    return out


def spill(path):
    t = (np.arange(128) + 0.5) / 128
    a = np.clip(1 - t, 0, 1) ** 2.2
    img = np.zeros((128, 8, 4), np.uint8)
    img[..., :3] = 255
    img[..., 3] = (a * 255 + 0.5).astype(np.uint8)[:, None]
    Image.fromarray(img, "RGBA").save(path)
    print("wrote", path, img.shape)


if __name__ == "__main__":
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "content/textures")
    out.mkdir(parents=True, exist_ok=True)
    save(base(), out / "floor_base.png")
    save(glow(), out / "floor_glow.png")
    spill(out / "wall_spill.png")
