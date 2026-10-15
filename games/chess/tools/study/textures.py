"""Procedural textures for the chess study scene.

Run with the host Python (numpy + Pillow):

    python3 games/chess/tools/study/textures.py [out_dir] [names...]

Every map is periodic unless it is a one-off layout (board, rug, dial, painting),
so tiled surfaces show no seams. Each material writes <name>_base_color.png
(sRGB), <name>_mr.png (glTF metallic-roughness: G roughness, B metallic) and
<name>_normal.png (OpenGL, +Y up).
"""

import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "build", "textures")
SERIF = "/usr/share/fonts/noto/NotoSerifDisplay-SemiBold.ttf"
SERIF_REGULAR = "/usr/share/fonts/noto/NotoSerifDisplay-Regular.ttf"


# ----------------------------------------------------------------------------
# Noise helpers (all periodic over the image)
# ----------------------------------------------------------------------------

def noise(shape, scale, seed, aniso=(1, 1)):
    """Band-limited periodic noise with features about `scale` pixels wide.

    aniso = (ly, lx) stretches features: a larger value makes them longer along that axis."""
    rng = np.random.default_rng(seed)
    h, w = shape
    white = rng.standard_normal(shape)
    fy = np.fft.fftfreq(h)[:, None] * aniso[0]
    fx = np.fft.fftfreq(w)[None, :] * aniso[1]
    # Penalising a frequency axis harder keeps features long along it.
    f = np.sqrt(fx * fx + fy * fy) * scale
    spectrum = np.fft.fft2(white) * np.exp(-(f ** 2) * 4.0)
    out = np.real(np.fft.ifft2(spectrum))
    out -= out.mean()
    out /= out.std() + 1e-9
    return out


def fbm(shape, scale, seed, octaves=4, aniso=(1, 1), gain=0.5):
    total = np.zeros(shape)
    amp = 1.0
    norm = 0.0
    for i in range(octaves):
        total += amp * noise(shape, scale / (2 ** i), seed + i * 101, aniso)
        norm += amp
        amp *= gain
    return total / norm


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def lerp(a, b, t):
    return a + (b - a) * t


def mix_color(c0, c1, t):
    c0 = np.asarray(c0, dtype=np.float64)
    c1 = np.asarray(c1, dtype=np.float64)
    return c0 + (c1 - c0) * t[..., None]


def srgb_to_linear(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(c, 0.0, 1.0)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(c, 1 / 2.4) - 0.055)


def hexc(h):
    h = h.lstrip("#")
    return srgb_to_linear([int(h[i:i + 2], 16) / 255.0 for i in (0, 2, 4)])


def normal_from_height(height, strength, wrap=True):
    if wrap:
        dx = (np.roll(height, -1, 1) - np.roll(height, 1, 1)) * 0.5
        dy = (np.roll(height, -1, 0) - np.roll(height, 1, 0)) * 0.5
    else:
        dy, dx = np.gradient(height)
    # Image rows grow downward, texture V grows upward: flip the Y slope.
    nx = -dx * strength
    ny = dy * strength
    nz = np.ones_like(height)
    length = np.sqrt(nx * nx + ny * ny + nz * nz)
    n = np.stack([nx / length, ny / length, nz / length], axis=-1)
    return n * 0.5 + 0.5


def save_rgb(path, linear_rgb, srgb=True):
    data = linear_to_srgb(linear_rgb) if srgb else np.clip(linear_rgb, 0, 1)
    Image.fromarray((data * 255.0 + 0.5).astype(np.uint8), "RGB").save(path, optimize=True)


def save_material(name, color_lin, rough, height, normal_strength, metal=None, wrap=True,
                  normal_downscale=1):
    os.makedirs(OUT, exist_ok=True)
    save_rgb(os.path.join(OUT, f"{name}_base_color.png"), color_lin)
    metal = np.zeros_like(rough) if metal is None else metal
    mr = np.stack([np.ones_like(rough), np.clip(rough, 0, 1), np.clip(metal, 0, 1)], -1)
    save_rgb(os.path.join(OUT, f"{name}_mr.png"), mr, srgb=False)
    if normal_downscale > 1:
        k = normal_downscale
        h, w = height.shape
        height = height[:h - h % k, :w - w % k].reshape(h // k, k, w // k, k).mean(axis=(1, 3))
        normal_strength /= k
    save_rgb(os.path.join(OUT, f"{name}_normal.png"),
             normal_from_height(height, normal_strength, wrap), srgb=False)
    print("wrote", name)


# ----------------------------------------------------------------------------
# Wood
# ----------------------------------------------------------------------------

def wood_fields(size, seed, rings, warp=0.6, fiber_scale=2.0, cycles_x=None):
    """Periodic wood: grain runs along image Y (texture V). Returns (ring, fiber, pores, tone)."""
    h, w = size
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    # Large slow warp gives the cathedral/flame figure; long along the grain.
    warp_field = fbm(size, w / 3.0, seed, 3, aniso=(4, 1)) * warp
    warp_fine = noise(size, w / 24.0, seed + 7, aniso=(8.33333, 1)) * 0.08
    g = xx / w * rings + warp_field + warp_fine
    ring = g - np.floor(g)
    fiber = noise(size, fiber_scale, seed + 11, aniso=(25, 1))
    pores = noise(size, 1.2, seed + 13, aniso=(6.66667, 1))
    tone = fbm(size, w / 2.0, seed + 17, 3)
    return ring, fiber, pores, tone


def wood(size, seed, early, late, rings=9, warp=0.6, late_width=0.35, fiber_amt=0.12,
         pore_amt=0.0, tone_amt=0.08, rough=0.32, rough_var=0.06, fiber_scale=2.0):
    ring, fiber, pores, tone = wood_fields(size, seed, rings, warp, fiber_scale)
    # Latewood band: sharp on one side, soft on the other.
    band = smoothstep(1.0 - late_width, 1.0, ring) * (1.0 - smoothstep(0.0, 0.04, ring)) \
        + (1.0 - smoothstep(0.0, 0.04, ring)) * 0.6
    band = np.clip(band, 0, 1)
    t = np.clip(band + fiber * fiber_amt + tone * tone_amt, 0, 1)
    color = mix_color(early, late, t)
    color *= (1.0 + tone[..., None] * tone_amt * 0.8)
    if pore_amt > 0:
        p = np.clip(pores - 1.6, 0, None) * pore_amt
        color *= (1.0 - p[..., None])
    height = band * 0.6 + fiber * 0.25 - np.clip(pores - 1.6, 0, None) * pore_amt * 2.0
    roughness = rough + (band - 0.5) * rough_var + fiber * 0.015
    return color, roughness, height


def piece_woods():
    size = (1024, 1024)
    # Boxwood: pale honey, almost invisible grain, satin lacquer.
    c, r, h = wood(size, 3, hexc("#e9d3a6"), hexc("#d2b07a"), rings=7, warp=0.9,
                   late_width=0.25, fiber_amt=0.06, tone_amt=0.06, rough=0.27, rough_var=0.04)
    save_material("boxwood", c, r, h, 1.2)
    # Ebony: near black with faint brown streaks, high polish.
    c, r, h = wood(size, 5, hexc("#1d1714"), hexc("#0b0908"), rings=13, warp=1.3,
                   late_width=0.5, fiber_amt=0.25, tone_amt=0.18, rough=0.2, rough_var=0.05)
    save_material("ebony", c, r, h, 1.0)


def table_wood():
    size = (2048, 2048)
    c, r, h = wood(size, 21, hexc("#7a3b22"), hexc("#3e1a0e"), rings=14, warp=1.1,
                   late_width=0.4, fiber_amt=0.14, pore_amt=0.15, tone_amt=0.12,
                   rough=0.24, rough_var=0.05)
    save_material("mahogany", c, r, h, 1.4)
    c, r, h = wood(size, 23, hexc("#6b4a33"), hexc("#3a2618"), rings=11, warp=0.8,
                   late_width=0.4, fiber_amt=0.14, pore_amt=0.2, tone_amt=0.1,
                   rough=0.42, rough_var=0.06)
    save_material("walnut_satin", c, r, h, 1.6)


# ----------------------------------------------------------------------------
# Chess board: one 4096 layout for the top face, coordinates included.
# ----------------------------------------------------------------------------

BOARD_SIZE = 0.52      # metres, whole top
SQUARE = 0.055         # metres
BORDER = (BOARD_SIZE - 8 * SQUARE) / 2


def sample_tile(tile, x0, y0, h, w, rotate=False):
    """Cut an h x w window out of a periodic tile, starting at (x0, y0)."""
    src = np.rot90(tile) if rotate else tile
    th, tw = src.shape[:2]
    ys = (np.arange(h) + y0) % th
    xs = (np.arange(w) + x0) % tw
    return src[np.ix_(ys, xs)]


def board():
    n = 4096
    px = n / BOARD_SIZE
    rng = np.random.default_rng(42)
    maple_c, maple_r, maple_h = wood((1024, 1024), 31, hexc("#ecd9b4"), hexc("#d9bf91"),
                                     rings=10, warp=1.4, late_width=0.2, fiber_amt=0.08,
                                     tone_amt=0.06, rough=0.22, rough_var=0.03, fiber_scale=1.6)
    # Curly figure: soft stripes across the grain, as in fiddleback maple.
    yy = np.arange(1024)[:, None]
    curl = np.sin(yy / 1024 * 2 * np.pi * 38 + noise((1024, 1024), 60, 37, (1, 3.3)) * 0.8)
    maple_c *= (1.0 + curl[..., None] * 0.045)
    walnut_c, walnut_r, walnut_h = wood((1024, 1024), 33, hexc("#6a432b"), hexc("#331f13"),
                                        rings=12, warp=1.0, late_width=0.45, fiber_amt=0.16,
                                        pore_amt=0.12, tone_amt=0.12, rough=0.22,
                                        rough_var=0.04)
    frame_c, frame_r, frame_h = wood((2048, 2048), 35, hexc("#4a2b1a"), hexc("#24130b"),
                                     rings=18, warp=1.3, late_width=0.45, fiber_amt=0.18,
                                     pore_amt=0.15, tone_amt=0.14, rough=0.22, rough_var=0.04)

    color = np.zeros((n, n, 3))
    rough = np.zeros((n, n))
    height = np.zeros((n, n))

    def put(region, c, r, h):
        y0, y1, x0, x1 = region
        color[y0:y1, x0:x1] = c
        rough[y0:y1, x0:x1] = r
        height[y0:y1, x0:x1] = h

    # Border frame: four mitred pieces with the grain running along each side.
    yy, xx = np.mgrid[0:n, 0:n]
    cx = (xx + 0.5) / n - 0.5
    cy = (yy + 0.5) / n - 0.5
    vertical = np.abs(cx) > np.abs(cy)  # left/right rails
    fc_v = sample_tile(frame_c, 0, 0, n, n)
    fr_v = sample_tile(frame_r, 0, 0, n, n)
    fh_v = sample_tile(frame_h, 0, 0, n, n)
    fc_h = sample_tile(frame_c, 300, 700, n, n, rotate=True)
    fr_h = sample_tile(frame_r, 300, 700, n, n, rotate=True)
    fh_h = sample_tile(frame_h, 300, 700, n, n, rotate=True)
    color[:] = np.where(vertical[..., None], fc_v, fc_h)
    rough[:] = np.where(vertical, fr_v, fr_h)
    height[:] = np.where(vertical, fh_v, fh_h)
    mitre = np.abs(np.abs(cx) - np.abs(cy)) < 0.6 / n
    height[mitre] -= 0.8

    # Squares
    b = BORDER * px
    s = SQUARE * px
    for rank in range(8):
        for file in range(8):
            x0 = int(round(b + file * s))
            x1 = int(round(b + (file + 1) * s))
            # Image row 0 is the far (black) edge: rank 8 at the top.
            y0 = int(round(b + (7 - rank) * s))
            y1 = int(round(b + (8 - rank) * s))
            light = (rank + file) % 2 == 1
            ox, oy = rng.integers(0, 1024, 2)
            if light:
                c = sample_tile(maple_c, ox, oy, y1 - y0, x1 - x0)
                r = sample_tile(maple_r, ox, oy, y1 - y0, x1 - x0)
                hh = sample_tile(maple_h, ox, oy, y1 - y0, x1 - x0)
                c = c * rng.uniform(0.95, 1.04)
            else:
                c = sample_tile(walnut_c, ox, oy, y1 - y0, x1 - x0)
                r = sample_tile(walnut_r, ox, oy, y1 - y0, x1 - x0)
                hh = sample_tile(walnut_h, ox, oy, y1 - y0, x1 - x0)
                c = c * rng.uniform(0.9, 1.08)
            put((y0, y1, x0, x1), c, r, hh)
            # Hairline veneer seams.
            height[y0:y1, x0] -= 0.7
            height[y0, x0:x1] -= 0.7
            color[y0:y1, x0] *= 0.82
            color[y0, x0:x1] *= 0.82

    # Maple stringing around the field and near the outer edge.
    inlay = np.zeros((n, n), dtype=bool)
    for inset, width in ((b - 0.0045 * px, 0.0016 * px), (0.006 * px, 0.0012 * px)):
        lo = inset
        hi = n - inset
        outer = (xx >= lo - width) & (xx < hi + width) & (yy >= lo - width) & (yy < hi + width)
        inner = (xx >= lo) & (xx < hi) & (yy >= lo) & (yy < hi)
        inlay |= outer & ~inner
    string_c = sample_tile(maple_c, 0, 0, n, n) * 1.02
    color[inlay] = string_c[inlay]
    rough[inlay] = 0.22

    # Coordinates as maple inlay: a-h along both rank edges, 1-8 along both file edges,
    # each turned to read from the nearer player.
    mask_img = Image.new("L", (n, n), 0)
    font = ImageFont.truetype(SERIF, int(0.0135 * px))
    def glyph(text, cx_px, cy_px, angle):
        tile = Image.new("L", (int(s), int(s)), 0)
        d = ImageDraw.Draw(tile)
        bbox = d.textbbox((0, 0), text, font=font, anchor="ls")
        tw = bbox[2] - bbox[0]
        # Centre on the x-height box so a-h and 1-8 share a baseline.
        th = font.getbbox("x", anchor="ls")[1]
        d.text((tile.width / 2 - tw / 2 - bbox[0], tile.height / 2 - th / 2), text, fill=255,
               font=font, anchor="ls")
        tile = tile.rotate(angle, resample=Image.BICUBIC)
        mask_img.paste(tile, (int(cx_px - tile.width / 2), int(cy_px - tile.height / 2)), tile)
    mid = b / 2 + 0.001 * px
    for i in range(8):
        cxp = b + (i + 0.5) * s
        glyph("abcdefgh"[i], cxp, n - mid, 0)        # white's edge (bottom)
        glyph("abcdefgh"[i], cxp, mid, 180)          # black's edge (top)
        cyp = b + (7 - i + 0.5) * s
        glyph(str(i + 1), mid, cyp, 0)               # white's left
        glyph(str(i + 1), n - mid, cyp, 180)         # black's left
    mask = np.asarray(mask_img.filter(ImageFilter.GaussianBlur(0.6)), dtype=np.float64) / 255.0
    letter_c = sample_tile(maple_c, 200, 100, n, n) * 1.06
    color = color * (1 - mask[..., None]) + letter_c * mask[..., None]
    height -= mask * 0.25

    # Satin lacquer: very even, a little fine orange-peel.
    rough = np.clip(rough * 0.6 + 0.12 + noise((n, n), 200, 78) * 0.015, 0.1, 0.5)
    save_material("board", color, rough, height, 1.1, wrap=False, normal_downscale=2)


# ----------------------------------------------------------------------------
# Floor: oak planks, periodic over the tile.
# ----------------------------------------------------------------------------

def floor():
    n = 2048
    tile_m = 2.4
    px = n / tile_m
    plank_w = 0.12 * px
    rows = int(round(n / plank_w))
    plank_w = n / rows
    rng = np.random.default_rng(9)
    oak_c, oak_r, oak_h = wood((1024, 1024), 51, hexc("#a8784a"), hexc("#6e4524"), rings=16,
                               warp=1.2, late_width=0.35, fiber_amt=0.16, pore_amt=0.25,
                               tone_amt=0.12, rough=0.3, rough_var=0.08, fiber_scale=1.4)
    # Planks run along image X here: rotate the tile so the grain follows them.
    oak_c = np.rot90(oak_c)
    oak_r = np.rot90(oak_r)
    oak_h = np.rot90(oak_h)
    color = np.zeros((n, n, 3))
    rough = np.zeros((n, n))
    height = np.zeros((n, n))
    for row in range(rows):
        y0 = int(round(row * plank_w))
        y1 = int(round((row + 1) * plank_w))
        x = rng.uniform(0, n)
        start = x
        while True:
            length = rng.uniform(0.6, 1.4) * px
            x1 = min(x + length, start + n)
            ox, oy = rng.integers(0, 1024, 2)
            seg = int(round(x1 - x))
            if seg <= 2:
                break
            xs = (np.arange(int(round(x)), int(round(x)) + seg)) % n
            tone = rng.uniform(0.82, 1.12)
            warm = rng.uniform(-0.04, 0.04)
            c = sample_tile(oak_c, ox, oy, y1 - y0, seg) * tone
            c[..., 0] *= 1 + warm
            c[..., 2] *= 1 - warm
            color[y0:y1][:, xs] = c
            rough[y0:y1][:, xs] = sample_tile(oak_r, ox, oy, y1 - y0, seg) + rng.uniform(-0.04, 0.04)
            height[y0:y1][:, xs] = sample_tile(oak_h, ox, oy, y1 - y0, seg)
            # End joint
            height[y0:y1, xs[0]] -= 2.5
            color[y0:y1, xs[0]] *= 0.55
            x = x1
            if x1 >= start + n:
                break
        height[y0, :] -= 3.0
        color[y0, :] *= 0.5
        if y0 + 1 < n:
            height[y0 + 1, :] -= 1.0
            color[y0 + 1, :] *= 0.85
    # Waxed oil finish with soft wear.
    wear = fbm((n, n), 300, 61, 3)
    rough = np.clip(rough + wear * 0.05, 0.18, 0.7)
    save_material("oak_floor", color, rough, height, 1.6)


# ----------------------------------------------------------------------------
# Table leather: a one-off panel with gold tooling round the edge.
# ----------------------------------------------------------------------------

LEATHER_PANEL = (0.70, 0.54)   # metres


def leather_panel():
    w = 2048
    h = int(round(w * LEATHER_PANEL[1] / LEATHER_PANEL[0]))
    px = w / LEATHER_PANEL[0]
    rng = np.random.default_rng(5)
    pebble = noise((h, w), 2.2, 91) * 0.6 + noise((h, w), 6.0, 92) * 0.4
    creases = np.clip(np.abs(noise((h, w), 40, 93, aniso=(1, 3))) - 1.4, 0, None)
    mottling = fbm((h, w), 260, 94, 4)
    base = mix_color(hexc("#1f3a2b"), hexc("#2c4d38"), np.clip(0.5 + mottling * 0.35, 0, 1))
    base *= (1.0 - creases[..., None] * 0.25 - np.clip(pebble, 0, None)[..., None] * 0.05)
    # Burnished, darker margin just inside the tooling.
    yy, xx = np.mgrid[0:h, 0:w]
    edge = np.minimum(np.minimum(xx, w - 1 - xx), np.minimum(yy, h - 1 - yy)) / px
    base *= (0.78 + 0.22 * smoothstep(0.0, 0.05, edge))[..., None]
    rough = 0.55 + pebble * 0.05 + creases * 0.1
    metal = np.zeros((h, w))
    height = pebble * 0.35 - creases * 0.5
    # Gold tooling: a double line and a dotted line, inset from the edge.
    tool = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(tool)
    for inset, width in ((0.012, 0.0022), (0.0175, 0.0009)):
        i = inset * px
        d.rectangle([i, i, w - 1 - i, h - 1 - i], outline=255, width=max(1, int(width * px)))
    i = 0.023 * px
    step = 0.006 * px
    r = 0.0011 * px
    x = i
    while x < w - i:
        for y in (i, h - 1 - i):
            d.ellipse([x - r, y - r, x + r, y + r], fill=255)
        x += step
    y = i
    while y < h - i:
        for x in (i, w - 1 - i):
            d.ellipse([x - r, y - r, x + r, y + r], fill=255)
        y += step
    m = np.asarray(tool.filter(ImageFilter.GaussianBlur(0.8)), dtype=np.float64) / 255.0
    gold = hexc("#c9a052") * (0.85 + 0.15 * np.clip(noise((h, w), 3, 95), -1, 1))[..., None]
    color = base * (1 - m[..., None]) + gold * m[..., None]
    rough = rough * (1 - m) + 0.32 * m
    metal = m
    height = height - m * 0.8
    save_material("leather_panel", color, rough, height, 1.2, metal=metal, wrap=False)


def velvet():
    n = 1024
    pile = noise((n, n), 1.5, 141) * 0.5 + noise((n, n), 30, 142, aniso=(4, 1)) * 0.5
    color = mix_color(hexc("#3a0f14"), hexc("#5a1a20"), np.clip(0.5 + pile * 0.3, 0, 1))
    rough = 0.85 + pile * 0.04
    save_material("velvet", color, rough, pile, 0.6)


def plaster():
    n = 1024
    trowel = fbm((n, n), 160, 151, 4)
    fine = noise((n, n), 1.5, 152)
    color = mix_color(hexc("#c9b99a"), hexc("#d8caad"), np.clip(0.5 + trowel * 0.4, 0, 1))
    rough = 0.82 + fine * 0.03
    save_material("plaster", color, rough, trowel * 1.5 + fine * 0.15, 0.6)


def painted_wood():
    n = 1024
    ring, fiber, pores, tone = wood_fields((n, n), 161, 12, 0.8, 2.0)
    brush = noise((n, n), 1.2, 162, aniso=(30, 1))
    color = mix_color(hexc("#20352e"), hexc("#26403a"), np.clip(0.5 + tone * 0.2, 0, 1))
    rough = 0.42 + brush * 0.04
    save_material("painted_green", color, rough, ring * 0.25 + brush * 0.25, 0.7)
    color = mix_color(hexc("#ece6da"), hexc("#f3eee4"), np.clip(0.5 + tone * 0.2, 0, 1))
    save_material("painted_white", color, rough, ring * 0.25 + brush * 0.25, 0.7)


def clock_dial():
    """Cream dial with numerals, minute track and the red flag sector before 12."""
    n = 1024
    img = Image.new("RGB", (n, n), (238, 231, 214))
    d = ImageDraw.Draw(img)
    c = n / 2
    font = ImageFont.truetype(SERIF, int(n * 0.105))
    # Red flag sector from 55 to 60 minutes.
    d.pieslice([c - n * 0.47, c - n * 0.47, c + n * 0.47, c + n * 0.47], -120, -90,
               fill=(172, 32, 30))
    d.ellipse([c - n * 0.40, c - n * 0.40, c + n * 0.40, c + n * 0.40], fill=(238, 231, 214))
    for minute in range(60):
        a = math.radians(minute * 6 - 90)
        r0 = n * (0.405 if minute % 5 else 0.37)
        r1 = n * 0.465
        width = 9 if minute % 5 == 0 else 3
        d.line([(c + r0 * math.cos(a), c + r0 * math.sin(a)),
                (c + r1 * math.cos(a), c + r1 * math.sin(a))], fill=(25, 22, 20), width=width)
    for hour in range(1, 13):
        a = math.radians(hour * 30 - 90)
        r = n * 0.29
        d.text((c + r * math.cos(a), c + r * math.sin(a)), str(hour), fill=(25, 22, 20),
               font=font, anchor="mm")
    d.ellipse([c - n * 0.495, c - n * 0.495, c + n * 0.495, c + n * 0.495], outline=(120, 110, 95),
              width=6)
    small = ImageFont.truetype(SERIF_REGULAR, int(n * 0.034))
    d.text((c, c + n * 0.15), "CHRONOMÈTRE D'ÉCHECS", fill=(90, 80, 70), font=small, anchor="mm")
    arr = srgb_to_linear(np.asarray(img, dtype=np.float64) / 255.0)
    os.makedirs(OUT, exist_ok=True)
    save_rgb(os.path.join(OUT, "dial_base_color.png"), arr)
    print("wrote dial")


# ----------------------------------------------------------------------------
# Persian rug (one-off layout, 2.6 x 1.8 m)
# ----------------------------------------------------------------------------

RUG = (2.6, 1.8)


def rug():
    w = 2600
    h = 1800
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    u = xx / w * RUG[0]          # metres
    v = yy / h * RUG[1]
    # Distance to the nearest edge, in metres; borders are bands of it.
    e = np.minimum(np.minimum(u, RUG[0] - u), np.minimum(v, RUG[1] - v))
    cx, cy = RUG[0] / 2, RUG[1] / 2
    dx, dy = u - cx, v - cy
    pal = {
        "red": hexc("#7a1a1c"), "deep": hexc("#4a0f14"), "navy": hexc("#1b2440"),
        "ivory": hexc("#d9c9a3"), "gold": hexc("#b98a3e"), "green": hexc("#2e4a3a"),
        "rose": hexc("#a8524a"), "black": hexc("#1a1414"),
    }
    color = np.zeros((h, w, 3)) + pal["red"]
    # Field lattice: small diamonds and rosettes on the red ground.
    gx = (u - cx) / 0.16
    gy = (v - cy) / 0.16
    diamond = np.abs(gx - np.round(gx)) + np.abs(gy - np.round(gy))
    color = np.where((diamond < 0.18)[..., None], pal["navy"], color)
    color = np.where(((diamond > 0.22) & (diamond < 0.27))[..., None], pal["ivory"], color)
    ox = (u - cx) / 0.16 + 0.5
    oy = (v - cy) / 0.16 + 0.5
    dot = np.hypot(ox - np.round(ox), oy - np.round(oy))
    color = np.where((dot < 0.09)[..., None], pal["gold"], color)
    # Central medallion: a lobed star.
    r = np.hypot(dx / 1.35, dy)
    th = np.arctan2(dy, dx / 1.35)
    lobe = 0.36 + 0.06 * np.cos(8 * th) + 0.02 * np.cos(16 * th)
    med = r < lobe
    color = np.where(med[..., None], pal["navy"], color)
    ring = (r > lobe * 0.62) & (r < lobe * 0.70)
    color = np.where((med & ring)[..., None], pal["ivory"], color)
    inner = r < lobe * 0.55
    petal = 0.13 + 0.05 * np.cos(6 * th)
    color = np.where(inner[..., None], pal["red"], color)
    color = np.where((inner & (r < petal))[..., None], pal["ivory"], color)
    color = np.where((inner & (r < petal * 0.55))[..., None], pal["gold"], color)
    color = np.where((np.abs(r - lobe) < 0.012)[..., None], pal["gold"], color)
    # Corner spandrels: quarter medallions in navy.
    for sx in (0, RUG[0]):
        for sy in (0, RUG[1]):
            rr = np.hypot((u - sx) / 1.3, v - sy)
            tt = np.arctan2(v - sy, (u - sx) / 1.3)
            sp = rr < 0.5 + 0.04 * np.cos(10 * tt)
            color = np.where((sp & (e > 0.27))[..., None], pal["navy"], color)
            color = np.where(((np.abs(rr - 0.5 - 0.04 * np.cos(10 * tt)) < 0.01) & (e > 0.27))[..., None],
                             pal["gold"], color)
    # Borders: guard stripes and a main band of rosettes on navy.
    def band(a, b, c):
        return ((e >= a) & (e < b))[..., None], c
    for mask, c in (band(0.0, 0.035, pal["deep"]), band(0.035, 0.05, pal["ivory"]),
                    band(0.05, 0.06, pal["red"]), band(0.06, 0.07, pal["gold"]),
                    band(0.07, 0.22, pal["navy"]), band(0.22, 0.23, pal["gold"]),
                    band(0.23, 0.245, pal["red"]), band(0.245, 0.26, pal["ivory"])):
        color = np.where(mask, c, color)
    # Rosettes and vine in the main band.
    along = np.where(np.minimum(u, RUG[0] - u) < np.minimum(v, RUG[1] - v), v, u)
    t = along / 0.2
    across = (e - 0.145) / 0.075
    ph = (t - np.floor(t)) - 0.5
    ros = np.hypot(ph * 2.0, across) < 0.55 + 0.12 * np.cos(np.arctan2(across, ph) * 6)
    in_band = (e >= 0.075) & (e < 0.215)
    color = np.where((ros & in_band)[..., None], pal["rose"], color)
    core = np.hypot(ph * 2.0, across) < 0.22
    color = np.where((core & in_band)[..., None], pal["gold"], color)
    vine = np.abs(across - 0.55 * np.sin(t * 2 * np.pi)) < 0.06
    color = np.where((vine & in_band & ~ros)[..., None], pal["ivory"], color)
    # Hand-knotted look: per-knot jitter, abrash (colour drift), wear and pile.
    knot = noise((h, w), 0.8, 201)
    abrash = noise((h, w), 220, 202, aniso=(0.3, 1))
    wear = np.clip(fbm((h, w), 120, 203, 4) - 0.9, 0, None) * np.clip(1.0 - e / 0.5, 0.3, 1.0)
    color = color * (1.0 + knot[..., None] * 0.08 + abrash[..., None] * 0.06)
    color = color * (1 - wear[..., None] * 0.18) + hexc("#b8a88a") * (wear[..., None] * 0.08)
    pile = knot * 0.5 + noise((h, w), 3.0, 204) * 0.5
    rough = 0.9 + pile * 0.03
    save_material("rug", color, rough, pile, 0.8, wrap=False)


# ----------------------------------------------------------------------------
# Book spines atlas: 14 spine designs + a pages strip.
# ----------------------------------------------------------------------------

def books():
    w, h = 2048, 1024
    cols = 16
    cw = w // cols
    rng = np.random.default_rng(301)
    img = Image.new("RGB", (w, h))
    mr = Image.new("RGB", (w, h), (255, 150, 0))
    d = ImageDraw.Draw(img)
    dm = ImageDraw.Draw(mr)
    leathers = ["#5a1418", "#1d2b4a", "#23402f", "#6b4426", "#2a1a14", "#7a2a1c", "#3d1f2e",
                "#8a6a3a", "#14301f", "#4a3020", "#1a1a20", "#6a1f22", "#2f3d52", "#5c4a2a"]
    gold = (201, 160, 82)
    for i in range(14):
        x0 = i * cw
        base = tuple(int(int(leathers[i][k:k + 2], 16)) for k in (1, 3, 5))
        d.rectangle([x0, 0, x0 + cw - 1, h - 1], fill=base)
        style = i % 4
        bands = [0.06, 0.1, 0.84, 0.9] if style != 2 else [0.08, 0.92]
        for b in bands:
            y = int(b * h)
            d.rectangle([x0, y, x0 + cw - 1, y + 10], fill=tuple(int(c * 0.6) for c in base))
            d.line([x0 + 6, y - 3, x0 + cw - 7, y - 3], fill=gold, width=3)
            d.line([x0 + 6, y + 14, x0 + cw - 7, y + 14], fill=gold, width=3)
            dm.line([x0 + 6, y - 3, x0 + cw - 7, y - 3], fill=(255, 90, 255), width=3)
            dm.line([x0 + 6, y + 14, x0 + cw - 7, y + 14], fill=(255, 90, 255), width=3)
        if style in (0, 1, 3):
            # Title label.
            lab = tuple(int(c) for c in ((30, 20, 15) if style != 1 else (110, 30, 25)))
            y0, y1 = int(0.2 * h), int(0.34 * h)
            d.rectangle([x0 + 12, y0, x0 + cw - 13, y1], fill=lab)
            d.rectangle([x0 + 12, y0, x0 + cw - 13, y1], outline=gold, width=2)
            dm.rectangle([x0 + 12, y0, x0 + cw - 13, y1], outline=(255, 90, 255), width=2)
            for k in range(3):
                yy = y0 + 22 + k * 30
                ww = rng.integers(40, cw - 40)
                d.line([x0 + cw // 2 - ww // 2, yy, x0 + cw // 2 + ww // 2, yy], fill=gold, width=7)
                dm.line([x0 + cw // 2 - ww // 2, yy, x0 + cw // 2 + ww // 2, yy], fill=(255, 90, 255), width=7)
        if style == 2:
            for k in range(5):
                yy = int((0.3 + k * 0.09) * h)
                d.ellipse([x0 + cw // 2 - 8, yy - 8, x0 + cw // 2 + 8, yy + 8], fill=gold)
                dm.ellipse([x0 + cw // 2 - 8, yy - 8, x0 + cw // 2 + 8, yy + 8], fill=(255, 90, 255))
    # Pages: cream with fine lines, in the last two columns.
    x0 = 14 * cw
    d.rectangle([x0, 0, w - 1, h - 1], fill=(222, 210, 182))
    for y in range(0, h, 3):
        c = 200 + int(rng.integers(-12, 8))
        d.line([x0, y, w - 1, y], fill=(c, c - 10, c - 32))
    dm.rectangle([x0, 0, w - 1, h - 1], fill=(255, 215, 0))
    arr = srgb_to_linear(np.asarray(img, dtype=np.float64) / 255.0)
    grain = noise((h, w), 1.5, 302)
    arr *= (1 + grain[..., None] * 0.05)
    os.makedirs(OUT, exist_ok=True)
    save_rgb(os.path.join(OUT, "books_base_color.png"), arr)
    mr_arr = np.asarray(mr, dtype=np.float64) / 255.0
    mr_arr[..., 1] = np.clip(mr_arr[..., 1] + grain * 0.03, 0, 1)
    save_rgb(os.path.join(OUT, "books_mr.png"), mr_arr, srgb=False)
    save_rgb(os.path.join(OUT, "books_normal.png"), normal_from_height(grain * 0.4, 1.0), srgb=False)
    print("wrote books")


def marble():
    n = 1024
    warp = fbm((n, n), 220, 401, 5) * 2.2
    yy, xx = np.mgrid[0:n, 0:n] / n
    t = np.sin((xx * 3 + yy * 2) * 2 * np.pi + warp * 3.0)
    vein = np.clip(1 - np.abs(t) * 6, 0, 1) ** 2
    fine = np.clip(1 - np.abs(np.sin((xx * 7 - yy * 5) * 2 * np.pi + fbm((n, n), 90, 402, 4) * 4)) * 14, 0, 1)
    cloud = fbm((n, n), 300, 403, 3)
    color = mix_color(hexc("#e9e4da"), hexc("#d6cfc2"), np.clip(0.5 + cloud * 0.5, 0, 1))
    color = mix_color(color.reshape(-1, 3), hexc("#8a8378"), (vein * 0.7 + fine * 0.35).reshape(-1)).reshape(n, n, 3)
    rough = 0.12 + vein * 0.05
    save_material("marble", color, rough, vein * 0.1, 0.3)


def painting():
    """Oil landscape at dusk for the frame over the mantel (1.2 x 0.85)."""
    w, h = 1536, 1088
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float64)
    u = xx / w
    v = yy / h      # 0 top
    sky = mix_color(hexc("#f1c27a"), hexc("#3a4a6a"), np.clip(1 - v / 0.62, 0, 1) ** 1.3)
    sun = np.exp(-((u - 0.68) ** 2 * 30 + (v - 0.5) ** 2 * 80))
    sky += hexc("#ffdca0") * sun[..., None] * 1.2
    clouds = np.clip(fbm((h, w), 120, 501, 5, aniso=(0.35, 1)) - 0.2, 0, 1)
    sky = sky * (1 - clouds[..., None] * 0.35) + hexc("#d68a5a") * clouds[..., None] * 0.35
    color = sky
    # Distant hills, nearer hills, lake.
    def ridge(base, amp, scale, seed):
        r = fbm((1, w), scale, seed, 4)[0]
        return base + r * amp
    far = ridge(0.56, 0.035, 300, 502)
    near = ridge(0.63, 0.05, 200, 503)
    color = np.where((v > far)[..., None], hexc("#6a6a80") * 0.9 + sky * 0.25, color)
    color = np.where((v > near)[..., None], hexc("#2f3a2a"), color)
    lake = (v > 0.72) & (v < 0.84)
    refl = sky[np.clip((2 * 0.72 - v) * h, 0, h - 1).astype(int), xx.astype(int)]
    color = np.where(lake[..., None], refl * 0.7 + hexc("#2a3442") * 0.3, color)
    shore = v >= 0.84
    color = np.where(shore[..., None], hexc("#2a2a1a") * (1 + fbm((h, w), 60, 504, 3)[..., None] * 0.3), color)
    # Trees on the left: dark masses with ragged tops.
    tree = fbm((h, w), 25, 505, 4)
    trees = (u < 0.3 + tree * 0.04) & (v > 0.32 + 0.15 * u + tree * 0.05) & (v < 0.86)
    color = np.where(trees[..., None], hexc("#1c2418") * (1 + tree[..., None] * 0.3), color)
    # Brush strokes and canvas.
    stroke = noise((h, w), 6, 506, aniso=(0.4, 2)) * 0.6 + noise((h, w), 2, 507) * 0.4
    color = color * (1 + stroke[..., None] * 0.05)
    weave = np.sin(xx * 1.7) * np.sin(yy * 1.7)
    color *= (0.9 + 0.1 * smoothstep(0.0, 1.0, color.mean(-1, keepdims=True) * 3))
    rough = 0.55 - stroke * 0.08
    save_material("painting", color, rough, stroke * 0.6 + weave * 0.15, 1.2, wrap=False)


# ----------------------------------------------------------------------------

def main():
    global OUT
    args = sys.argv[1:]
    if args and (os.sep in args[0] or args[0].startswith(".")):
        OUT = args.pop(0)
    jobs = {
        "pieces": piece_woods,
        "table": table_wood,
        "board": board,
        "floor": floor,
        "leather": leather_panel,
        "velvet": velvet,
        "plaster": plaster,
        "painted": painted_wood,
        "dial": clock_dial,
        "rug": rug,
        "books": books,
        "marble": marble,
        "painting": painting,
    }
    for name in args or jobs:
        jobs[name]()


if __name__ == "__main__":
    main()
