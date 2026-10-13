import sys
import numpy as np
from PIL import Image, ImageFilter

out = sys.argv[1]
rng = np.random.default_rng(7)

def save(a, name):
    a = np.clip(a, 0, 1)
    Image.fromarray((a * 255).astype(np.uint8)).save(f"{out}/{name}")

def smooth_noise(h, w, cells, seed):
    r = np.random.default_rng(seed)
    g = r.random((cells, cells))
    im = Image.fromarray((g * 255).astype(np.uint8)).resize((w, h), Image.BICUBIC)
    return np.asarray(im).astype(np.float32) / 255

# ---- brick: greyscale-white bevelled tile, tinted by the material colour
def brick(w=192, h=80):
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    base = 0.86 + 0.06 * (rng.random((h, w)) - 0.5)
    base += 0.05 * (1 - y / h)                      # glossy top
    b = 7
    dl, dr, dt, db = x, w - 1 - x, y, h - 1 - y
    edge = np.minimum(np.minimum(dl, dr), np.minimum(dt, db))
    img = base.copy()
    light = (dt < b) & (dt <= dl) & (dt <= dr)
    light |= (dl < b) & (dl < dt) & (dl <= db)
    dark = (db < b) & (db <= dl) & (db <= dr)
    dark |= (dr < b) & (dr < db) & (dr <= dt)
    img[light] = 1.0 - 0.04 * (dt[light] / b)
    img[dark] = 0.52 + 0.05 * (db[dark] / b)
    img[edge == 0] *= 0.7                            # outer groove
    stripe = np.exp(-((y - h * 0.32) / 3.0) ** 2) * 0.10
    img[edge >= b] += stripe[edge >= b]              # specular streak
    rgb = np.stack([img, img, img], -1)
    return rgb

save(brick(), "brick.png")

# ---- paddle: gunmetal with glowing cyan core strip and end caps
def paddle(w=256, h=64):
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    metal = 0.30 + 0.10 * np.sin(x * 0.9) * 0.2 + 0.05 * rng.random((h, w))
    metal += 0.25 * np.exp(-((y - h * 0.22) / 6.0) ** 2)
    rgb = np.stack([metal * 0.8, metal * 0.88, metal * 1.05], -1)
    core = np.exp(-((y - h * 0.55) / 4.5) ** 2)
    taper = np.clip(1 - (np.abs(x - w / 2) / (w * 0.40)) ** 6, 0, 1)
    glow = (core * taper)[..., None] * np.array([0.1, 0.9, 1.0])
    rgb = rgb * (1 - 0.6 * (core * taper)[..., None]) + glow
    cap = (x < 22) | (x > w - 23)
    rgb[cap] = rgb[cap] * 0.5 + np.array([0.55, 0.12, 0.15]) * 0.8 * (0.8 + 0.2 * np.sin(y[cap] * 0.5))[..., None]
    edge = np.minimum(np.minimum(x, w - 1 - x), np.minimum(y, h - 1 - y))
    rgb[edge < 3] *= 0.45
    for rx in (10, w - 11):
        d = (x - rx) ** 2 + (y - h * 0.5) ** 2
        rgb[d < 12] = 0.85
    return rgb

save(paddle(), "paddle.png")

# ---- background: nebula + stars + faint neon grid
def background(w=1024, h=896):
    n = 0.5 * smooth_noise(h, w, 6, 1) + 0.3 * smooth_noise(h, w, 14, 2) + 0.2 * smooth_noise(h, w, 40, 3)
    n = (n - n.min()) / (n.max() - n.min())
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    t = y / h
    rgb = np.zeros((h, w, 3), np.float32)
    top = np.array([0.020, 0.025, 0.075]); bot = np.array([0.045, 0.015, 0.075])
    rgb += top * (1 - t)[..., None] + bot * t[..., None]
    rgb += (n ** 2.2)[..., None] * np.array([0.10, 0.07, 0.20])
    rgb += (smooth_noise(h, w, 5, 9) ** 3)[..., None] * np.array([0.00, 0.10, 0.14])
    stars = rng.random((h, w)) > 0.9985
    bright = rng.random((h, w)) * 0.8 + 0.2
    im = Image.fromarray((stars * bright * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.8))
    s = np.asarray(im).astype(np.float32) / 255 * 4
    rgb += s[..., None] * np.array([0.8, 0.9, 1.0])
    gx = (x % 56 < 1.5); gy = (y % 56 < 1.5)
    grid = (gx | gy).astype(np.float32)
    rgb += grid[..., None] * np.array([0.02, 0.07, 0.12]) * (0.5 + 0.5 * n)[..., None]
    v = 1 - 0.55 * (((x - w / 2) / (w / 2)) ** 2 + ((y - h / 2) / (h / 2)) ** 2) / 2
    return np.clip(rgb * 1.8, 0, 1) * v[..., None]

save(background(), "starfield.png")

# ---- wall: brushed steel with a neon inner rail
def wall(w=128, h=128):
    streak = smooth_noise(h, w, 2, 5)
    r = np.random.default_rng(11)
    lines = np.asarray(Image.fromarray((r.random((h, 1)) * 255).astype(np.uint8)).resize((w, h), Image.NEAREST)).astype(np.float32) / 255
    m = 0.32 + 0.18 * lines + 0.08 * streak
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    m += 0.12 * np.exp(-((y - 20) / 7.0) ** 2)
    rgb = np.stack([m * 0.85, m * 0.92, m * 1.08], -1)
    for cx in (16, 80):
        for cy in (28, 100):
            d = (x - cx) ** 2 + (y - cy) ** 2
            rgb[d < 14] = 0.8
            rgb[(d >= 14) & (d < 26)] *= 0.55
    return rgb

save(wall(), "wall.png")

# ---- floor sheen for the backboard frame (flat dark panel w/ subtle scratches)
print("ok")
