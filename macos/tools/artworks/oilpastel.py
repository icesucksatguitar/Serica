"""Re-paints a picture in oil pastel, stroke by stroke.

An oil-pastel picture is built from thick, waxy strokes laid along the
forms, coarse first and finer where there is detail; each stroke is one
slightly different colour (the stick, the pressure), strokes overlap and
smear, and in light passages the paper's tooth shows through as broken
specks in the grooves of the strokes.

usage: python -P oilpastel2.py IN OUT_BASENAME crop_l crop_t crop_r crop_b [seed]
(crops are fractions of the source to cut off each side)
"""
import math
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageEnhance, ImageFilter

W = 2560
PAPER = (236, 228, 210)


def box(a, r):
    p = np.pad(a, ((r + 1, r), (r + 1, r)) + ((0, 0),) * (a.ndim - 2), mode='edge')
    c = p.cumsum(0).cumsum(1)
    k = 2 * r + 1
    return (c[k:, k:] - c[:-k, k:] - c[k:, :-k] + c[:-k, :-k]) / (k * k)


def sobel(a):
    p = np.pad(a, 1, mode='edge')
    gx = (p[:-2, 2:] + 2 * p[1:-1, 2:] + p[2:, 2:]) - (p[:-2, :-2] + 2 * p[1:-1, :-2] + p[2:, :-2])
    gy = (p[2:, :-2] + 2 * p[2:, 1:-1] + p[2:, 2:]) - (p[:-2, :-2] + 2 * p[:-2, 1:-1] + p[:-2, 2:])
    return gx, gy


def paint(path, out_base, crop, seed=7):
    rng = np.random.default_rng(seed)
    src = Image.open(path).convert('RGB')
    l, t, r, b = crop
    src = src.crop((round(src.width * l), round(src.height * t), round(src.width * (1 - r)), round(src.height * (1 - b))))
    H = round(src.height * W / src.width)
    src = src.resize((W, H), Image.LANCZOS)
    src = ImageEnhance.Color(src).enhance(1.42)
    src = ImageEnhance.Contrast(src).enhance(1.05)
    src = ImageEnhance.Brightness(src).enhance(1.04)
    ref = np.asarray(src).astype(np.float32)

    # Orientation along the forms (structure tensor), a diagonal hatch where flat.
    lum = ref @ np.array([0.299, 0.587, 0.114], dtype=np.float32) / 255.0
    gx, gy = sobel(box(lum, 2))
    jxx, jyy, jxy = box(gx * gx, 12), box(gy * gy, 12), box(gx * gy, 12)
    theta = 0.5 * np.arctan2(2 * jxy, jxx - jyy) + math.pi / 2
    coherence = np.sqrt((jxx - jyy) ** 2 + 4 * jxy ** 2) / (jxx + jyy + 1e-4)
    detail = box(np.sqrt(gx * gx + gy * gy), 6)
    detail = detail / (np.percentile(detail, 97) + 1e-6)

    # Underlayer: the picture very soft, so no bare paper between strokes.
    canvas = src.filter(ImageFilter.GaussianBlur(14))
    draw = ImageDraw.Draw(canvas)

    def layer(brush, spacing, only_detail=None):
        blurred = np.asarray(src.filter(ImageFilter.GaussianBlur(brush * 0.45))).astype(np.float32)
        ys = np.arange(spacing / 2, H, spacing)
        xs = np.arange(spacing / 2, W, spacing)
        pts = [(x + rng.uniform(-spacing / 2, spacing / 2), y + rng.uniform(-spacing / 2, spacing / 2)) for y in ys for x in xs]
        rng.shuffle(pts)
        for x, y in pts:
            xi, yi = int(min(W - 1, max(0, x))), int(min(H - 1, max(0, y)))
            if only_detail is not None and detail[yi, xi] < only_detail:
                continue
            a = theta[yi, xi] if coherence[yi, xi] > 0.25 else math.radians(-38) + rng.normal(0, 0.35)
            a += rng.normal(0, 0.12)
            length = brush * rng.uniform(2.2, 4.2)
            dx, dy = math.cos(a) * length / 2, math.sin(a) * length / 2
            c = blurred[yi, xi]
            # Each stick and pressure gives a slightly different colour.
            c = c * rng.uniform(0.92, 1.08) + rng.normal(0, 7.0, 3)
            col = tuple(int(v) for v in np.clip(c, 0, 255))
            w = max(2, int(brush * rng.uniform(0.75, 1.1)))
            # A slightly bent stroke: two segments.
            bend = rng.normal(0, 0.18)
            mx, my = x + math.cos(a + math.pi / 2) * bend * w, y + math.sin(a + math.pi / 2) * bend * w
            pts_line = [(x - dx, y - dy), (mx, my), (x + dx, y + dy)]
            draw.line(pts_line, fill=col, width=w, joint='curve')
            rr = w / 2
            for px, py in (pts_line[0], pts_line[-1]):
                draw.ellipse((px - rr, py - rr, px + rr, py + rr), fill=col)

    layer(22, 16)
    layer(12, 9)
    layer(6, 5, only_detail=0.35)
    layer(3.5, 3, only_detail=0.75)

    out = np.asarray(canvas).astype(np.float32) / 255.0
    lum2 = out @ np.array([0.299, 0.587, 0.114], dtype=np.float32)

    # Wax relief: fine streaks along the strokes catch the light.
    streak = rng.standard_normal((H, W)).astype(np.float32)
    # Smear the noise along the local direction (a few steps of advection).
    vx, vy = np.cos(theta).astype(np.float32), np.sin(theta).astype(np.float32)
    ys, xs = np.mgrid[0:H, 0:W].astype(np.float32)
    acc = streak.copy()
    for s in range(1, 7):
        for sign in (1, -1):
            xx = np.clip(xs + vx * s * 1.5 * sign, 0, W - 1).astype(np.int32)
            yy = np.clip(ys + vy * s * 1.5 * sign, 0, H - 1).astype(np.int32)
            acc += streak[yy, xx]
    streak = acc / acc.std()

    # Paper tooth in the grooves, mostly in light passages.
    grain = rng.standard_normal((H, W)).astype(np.float32)
    grain = (grain + 0.8 * streak) / 1.3
    light = np.clip((lum2 - 0.45) / 0.5, 0, 1)
    skip = np.clip((grain - 1.05) * 0.6, 0, 1) * (0.3 + 0.7 * light) * 0.8
    paper = np.array(PAPER, dtype=np.float32) / 255.0
    out = out * (1 + 0.035 * np.clip(streak, -2.5, 2.5))[..., None]
    out = out * (1 - skip[..., None]) + paper * skip[..., None]

    res = Image.fromarray((np.clip(out, 0, 1) * 255 + 0.5).astype(np.uint8), 'RGB')
    res = res.filter(ImageFilter.UnsharpMask(radius=1.0, percent=30, threshold=3))
    res.save(out_base + '.webp', 'WEBP', quality=80, method=6)
    res.save(out_base + '.avif', 'AVIF', quality=54, speed=4)
    res.resize((640, round(640 * H / W)), Image.LANCZOS).save(out_base + '-thumb.webp', 'WEBP', quality=78, method=6)
    return res.size


if __name__ == '__main__':
    crop = tuple(float(v) for v in sys.argv[3:7])
    seed = int(sys.argv[7]) if len(sys.argv) > 7 else 7
    print(paint(sys.argv[1], sys.argv[2], crop, seed))
