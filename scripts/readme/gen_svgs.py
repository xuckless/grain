"""Generates the README SVGs in Grain's dot-grain style. Run: python3 -I scripts/readme/gen_svgs.py docs/readme"""
import math, sys, os

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)

GROUND, SURFACE, RAISED = "#050506", "#0E0E11", "#17171B"
TEXT, MUTED, DIM = "#F2F2F0", "#9A9AA2", "#5E5E66"
HUE = {"Vocals": "#F0449A", "Drums": "#FF6B3D", "Bass": "#4F7DFF",
       "Guitar": "#3FD9D0", "Piano": "#B6E34B", "Other": "#E7D6B4"}
HIT, NEAR, MISS = "#FFFFFF", "#FFB547", "#8E6BFF"
FONT = "ui-sans-serif, -apple-system, 'Segoe UI', Helvetica, Arial, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"


def dot(x, y, r, c, o=1.0):
    op = f' fill-opacity="{o:.2f}"' if o < 1 else ""
    return f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.2f}" fill="{c}"{op}/>'


def text(x, y, s, size=13, c=TEXT, anchor="start", weight=400, family=FONT):
    return (f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" font-weight="{weight}" '
            f'fill="{c}" text-anchor="{anchor}">{s}</text>')


def frame(w, h, body, title):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
            f'role="img" aria-label="{title}"><title>{title}</title>'
            f'<rect width="{w}" height="{h}" rx="16" fill="{GROUND}"/>{body}</svg>')


def save(name, svg):
    with open(os.path.join(OUT, name), "w") as f:
        f.write(svg)


def noise(i, seed):
    return 0.5 + 0.5 * math.sin(i * 0.37 + seed) * math.cos(i * 0.113 + seed * 2.1)


# --- logo: dot orb + wordmark -----------------------------------------------------------
def orb(cx, cy, R, step, c):
    out = []
    y = cy - R
    while y <= cy + R:
        x = cx - R
        while x <= cx + R:
            dx, dy = x - cx, y - cy
            r = math.hypot(dx, dy)
            th = math.atan2(dy, dx)
            edge = R * (0.86 + 0.07 * math.sin(4 * th) + 0.04 * math.sin(7 * th + 1))
            if r <= edge:
                wave = 0.5 + 0.5 * math.sin(r / step * 0.55)
                soft = 1 - max(0, (r - (edge - 4 * step)) / (4 * step)) * 0.55
                d = min(step * 0.49, (0.1 + 0.28 * wave) * step * soft)
                out.append(dot(x, y, max(0.35, d), c))
            x += step
        y += step
    return "".join(out)


def logo(c):
    w, h = 340, 110
    body = orb(55, 55, 46, 5.2, c) + text(122, 74, "grain", 58, c, weight=500)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
            f'role="img" aria-label="Grain"><title>Grain</title>{body}</svg>')


save("logo-dark.svg", logo(TEXT))
save("logo-light.svg", logo(GROUND))

# --- stems: six dot-waveform lanes --------------------------------------------------------
W, H = 720, 330
b = []
lanes = list(HUE.items())
for li, (name, c) in enumerate(lanes):
    y0 = 34 + li * 48
    b.append(f'<rect x="16" y="{y0 - 20}" width="{W - 32}" height="40" rx="10" fill="{SURFACE}"/>')
    b.append(dot(36, y0, 4, c))
    b.append(text(50, y0 + 4.5, name, 13))
    activity = {"Vocals": 0.9, "Drums": 0.8, "Bass": 0.55, "Guitar": 0.6, "Piano": 0.35, "Other": 0.45}[name]
    for i in range(110):
        x = 140 + i * 5.2
        a = noise(i, li * 1.9) * activity
        n = max(1, round(a * 3.4))
        for k in range(-n, n + 1):
            b.append(dot(x, y0 + k * 4.4, 1.25 - abs(k) / (n + 1) * 0.35, c))
b.append(f'<rect x="402" y="10" width="2" height="{H - 24}" fill="{TEXT}"/>')
save("stems.svg", frame(W, H, "".join(b), "A song split into six coloured stems: vocals, drums, bass, guitar, piano and other, each drawn as a dot waveform"))

# --- karaoke: note lane, pitch trail, lyric wipe ---------------------------------------------
W, H = 720, 300
b = []
notes = [(40, 120, 150), (170, 90, 130), (270, 150, 120), (395, 70, 110), (475, 130, 140), (620, 80, 120)]
head = 360
for (x, wdt, y) in notes:
    passed = x + wdt < head
    col = HIT if passed and y != 120 else (NEAR if passed else DIM)
    b.append(f'<rect x="{x}" y="{y - 7}" width="{wdt}" height="14" rx="7" fill="{RAISED}"/>')
    if passed or x < head:
        filled = min(wdt, head - x)
        b.append(f'<rect x="{x}" y="{y - 7}" width="{filled}" height="14" rx="7" fill="{col}" fill-opacity="0.85"/>')
# pitch trail
pts = []
for i in range(0, head - 30, 3):
    xx = 30 + i
    target = next((y for (x, wdt, y) in notes if x <= xx <= x + wdt), None)
    yy = (target if target else 120) + 4 * math.sin(i * 0.21) + (6 if 170 <= xx <= 260 else 0)
    pts.append((xx, yy))
for j, (xx, yy) in enumerate(pts):
    age = j / len(pts)
    b.append(dot(xx, yy, 0.8 + 1.6 * age, "#F0449A", 0.25 + 0.75 * age))
b.append(dot(head, pts[-1][1], 6, TEXT))
b.append(f'<rect x="{head}" y="20" width="2" height="200" fill="{TEXT}" fill-opacity="0.6"/>')
# lyrics with wipe
b.append(text(360, 258, "Sing it back, every note in place", 22, MUTED, "middle", 500))
b.append('<clipPath id="wipe"><rect x="150" y="230" width="190" height="40"/></clipPath>')
b.append(f'<g clip-path="url(#wipe)">{text(360, 258, "Sing it back, every note in place", 22, TEXT, "middle", 500)}</g>')
b.append(text(690, 40, "x12 streak", 13, HIT, "end", 500, MONO))
save("karaoke.svg", frame(W, H, "".join(b), "Karaoke lane: reference notes as bars, your live pitch as a dot trail, notes fill as you hit them, and the lyric line wipes in time"))

# --- pitch editor: blobs, ghost notes, cents badges ----------------------------------------
W, H = 720, 300
b = []
for k in range(7):
    yy = 40 + k * 36
    b.append(f'<line x1="20" y1="{yy}" x2="{W - 20}" y2="{yy}" stroke="{TEXT}" stroke-opacity="0.08"/>')
blobs = [(40, 150, 112, 106, "+6¢"), (210, 120, 148, 148, "0¢"), (350, 160, 76, 92, "-23¢"), (530, 150, 184, 184, "+2¢")]
for (x, wdt, ghost_y, y, badge) in blobs:
    b.append(f'<rect x="{x}" y="{ghost_y - 11}" width="{wdt}" height="22" rx="11" fill="none" stroke="{MUTED}" stroke-dasharray="3 4" stroke-opacity="0.7"/>')
    for i in range(int(wdt / 6)):
        xx = x + 4 + i * 6
        amp = 6 + 5 * math.sin(i / max(1, wdt / 6) * math.pi)
        for k in (-1, 0, 1):
            b.append(dot(xx, y + k * amp * 0.55, 1.6 if k == 0 else 1.2, "#F0449A", 1 if k == 0 else 0.6))
    col = HIT if badge in ("0¢", "+2¢") else (NEAR if badge == "+6¢" else MISS)
    b.append(f'<rect x="{x + wdt / 2 - 22}" y="{y - 38}" width="44" height="20" rx="6" fill="{RAISED}"/>')
    b.append(text(x + wdt / 2, y - 24, badge, 11, col, "middle", 500, MONO))
b.append(text(W - 24, H - 18, "dashed = original melody", 12, MUTED, "end"))
save("pitch.svg", frame(W, H, "".join(b), "Pitch editor: your notes as dot blobs over dashed outlines of the original melody, each with a cents badge showing how far off it is"))

# --- chain: master take -> steps -> export ---------------------------------------------------
W, H = 720, 210
b = []
b.append(f'<rect x="24" y="70" width="150" height="70" rx="12" fill="{RAISED}"/>')
b.append(text(99, 100, "Take 1 ★", 15, TEXT, "middle", 500))
b.append(text(99, 122, "raw, never changed", 11, MUTED, "middle"))
steps = [("Pitch edits", True), ("Auto-tune", True), ("Clean up", False), ("Vocal chain", True)]
for i, (name, on) in enumerate(steps):
    x = 200 + i * 112
    b.append(f'<rect x="{x}" y="78" width="100" height="54" rx="10" fill="{SURFACE}" stroke="{TEXT if on else DIM}" stroke-opacity="{0.6 if on else 0.4}"/>')
    b.append(text(x + 50, 103, name, 12, TEXT if on else DIM, "middle", 500))
    b.append(text(x + 50, 120, "on" if on else "bypassed", 10, MUTED if on else DIM, "middle", 400, MONO))
for x in (180, 304, 416, 528, 640):
    b.append(f'<path d="M {x - 4} 105 l 8 0" stroke="{MUTED}"/>')
b.append(f'<rect x="652" y="78" width="48" height="54" rx="10" fill="{TEXT}"/>')
b.append(text(676, 110, "Export", 10, GROUND, "middle", 600))
b.append(text(W / 2, 176, "Every step can be bypassed and edited. ⌘Z undoes anything.", 13, MUTED, "middle"))
save("chain.svg", frame(W, H, "".join(b), "Your master take flows through pitch edits, auto-tune, clean up and the vocal chain, each switchable, into the export"))

# --- pipeline -------------------------------------------------------------------------------
W, H = 720, 150
b = []
stages = ["Import", "Separate", "Analyse", "Sing &amp; score", "Shape", "Export"]
for i, s in enumerate(stages):
    x = 24 + i * 115
    b.append(f'<rect x="{x}" y="45" width="100" height="50" rx="12" fill="{RAISED if i % 2 == 0 else SURFACE}"/>')
    b.append(text(x + 50, 75, s, 13, TEXT, "middle", 500))
    if i:
        b.append(f'<path d="M {x - 12} 70 l 9 0" stroke="{MUTED}"/>')
cols = list(HUE.values())
for i in range(36):
    b.append(dot(160 + i * 3.4, 112 + 4 * math.sin(i), 1.6, cols[i % 6]))
save("pipeline.svg", frame(W, H, "".join(b), "Import a song, separate it into stems, analyse tempo, key and melody, sing and score, shape your voice, export the mix"))
print("ok")
