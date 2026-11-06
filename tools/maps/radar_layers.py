"""Draws the radar's base map as layers (`RPML`, rp_weather::maplayers) for a
place and zoom: R184 step 1, docs/research/sheets/r184-radar-map-looks.md.

    radar_layers.py <maps dir> <font> <out file> <lat> <lon> <zoom>

The view is the device's (rp_weather::tiles::View::centred): 480 x 462
pixels of web Mercator at `zoom`, centred on the place. Data: Natural Earth
(public domain, `<maps dir>/ne/*.geojson`) and the open elevation tiles
(terrarium PNGs, fetched once into `<maps dir>/terrain/`). No colours here:
the device colours the layers in each look, so the looks live in one place
(rp_weather::maplayers::palette).

Everything is drawn at twice the size and averaged down, so each layer's
byte is how much of the pixel it covers (anti-aliased edges)."""
import json, math, os, pathlib, sys, urllib.request, zlib
import numpy as np
from PIL import Image, ImageDraw, ImageFont

maps, fontp, out = pathlib.Path(sys.argv[1]), sys.argv[2], pathlib.Path(sys.argv[3])
LAT, LON, Z = float(sys.argv[4]), float(sys.argv[5]), int(sys.argv[6])
W, H, S = 480, 462, 2
WORLD = 256 * 2 ** Z
# The layers, in the file's order (maplayers::Layer).
LAND, SHADE, RIVER, BORDER, ROAD, COAST, HALO, TEXT = range(8)


def merc(lon, lat):
    x = (lon + 180) / 360 * WORLD
    r = math.radians(max(-85.0, min(85.0, lat)))
    y = (1 - math.log(math.tan(r) + 1 / math.cos(r)) / math.pi) / 2 * WORLD
    return x, y


# The view's top left, as the device works it out (whole pixels).
CX, CY = merc(LON, LAT)
X0, Y0 = math.floor(CX) - W // 2, math.floor(CY) - H // 2


def px(lon, lat):
    x, y = merc(lon, lat)
    return ((x - X0) * S, (y - Y0) * S)


def lonlat(x, y):
    lon = x / WORLD * 360 - 180
    lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / WORLD))))
    return lon, lat


LON0, LAT1 = lonlat(X0 - 64, Y0 - 64)
LON1, LAT0 = lonlat(X0 + W + 64, Y0 + H + 64)


def inbox(coords):
    xs = [c[0] for c in coords]
    ys = [c[1] for c in coords]
    return not (max(xs) < LON0 or min(xs) > LON1 or max(ys) < LAT0 or min(ys) > LAT1)


def load(name):
    return json.load(open(maps / 'ne' / (name + '.geojson'), encoding='utf-8'))['features']


def rings(geom):
    if not geom:
        return []
    t, c = geom['type'], geom['coordinates']
    return [c] if t == 'Polygon' else c if t == 'MultiPolygon' else []


def lines(geom):
    if not geom:
        return []
    t, c = geom['type'], geom['coordinates']
    return [c] if t == 'LineString' else c if t == 'MultiLineString' else []


def canvas():
    return Image.new('L', (W * S, H * S), 0)


def down(img):
    """Twice the size averaged down: each pixel's coverage, 0-255."""
    return np.asarray(img.resize((W, H), Image.BOX), dtype=np.uint8)


def polys(img, ps, holes=True):
    d = ImageDraw.Draw(img)
    for p in ps:
        if len(p[0]) > 2:
            d.polygon([px(*c[:2]) for c in p[0]], fill=255)
        if holes:
            for h in p[1:]:
                if len(h) > 2:
                    d.polygon([px(*c[:2]) for c in h], fill=0)


land = [p for f in load('ne_10m_land') + load('ne_10m_minor_islands') for p in rings(f['geometry']) if inbox(p[0])]
lakes = [p for f in load('ne_10m_lakes') for p in rings(f['geometry']) if inbox(p[0])]
layers = np.zeros((8, H, W), np.uint8)

# Land, less the lakes.
m = canvas()
polys(m, land)
d = ImageDraw.Draw(m)
for p in lakes:
    if len(p[0]) > 2:
        d.polygon([px(*c[:2]) for c in p[0]], fill=0)
landm = down(m)
layers[LAND] = landm
# The coast: the land's edge, a pixel wide.
edge = np.zeros((H, W), np.int32)
lm = landm.astype(np.int32)
for dy, dx in ((0, 1), (1, 0), (0, -1), (-1, 0)):
    edge = np.maximum(edge, np.abs(lm - np.roll(np.roll(lm, dy, 0), dx, 1)))
layers[COAST] = np.clip(edge, 0, 255).astype(np.uint8)

# Rivers, borders, roads: lines at their widths (major roads 2 px whole,
# minor 1 px at half).
def strokes(layer, items, width, value=255):
    img = canvas()
    d = ImageDraw.Draw(img)
    for l in items:
        if len(l) > 1:
            d.line([px(*c[:2]) for c in l], fill=value, width=width * S)
    layers[layer] = np.maximum(layers[layer], down(img))


strokes(RIVER, [l for f in load('ne_10m_rivers_lake_centerlines') for l in lines(f['geometry']) if inbox(l)], 1)
strokes(BORDER, [l for f in load('ne_10m_admin_1_states_provinces_lines') for l in lines(f['geometry']) if inbox(l)], 1)
roads = [(f['properties'].get('type', ''), l) for f in load('ne_10m_roads') for l in lines(f['geometry']) if inbox(l)]
strokes(ROAD, [l for t, l in roads if 'Major' not in t], 1, 128)
strokes(ROAD, [l for t, l in roads if 'Major' in t], 2)

# The terrain's light, from the open elevation tiles at this zoom
# (terrarium: height = R * 256 + G + B / 256 - 32768 metres).
def tile(z, x, y):
    p = maps / 'terrain' / f'{z}_{x}_{y}.png'
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        url = f'https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png'
        req = urllib.request.Request(url, headers={'User-Agent': 'RetroPane-dev-maps'})
        p.write_bytes(urllib.request.urlopen(req, timeout=20).read())
    a = np.asarray(Image.open(p).convert('RGB'), dtype=np.float64)
    return a[..., 0] * 256 + a[..., 1] + a[..., 2] / 256 - 32768


n = 2 ** Z
tx0, ty0 = X0 // 256, Y0 // 256
tx1, ty1 = (X0 + W - 1) // 256, (Y0 + H - 1) // 256
mosaic = np.zeros(((ty1 - ty0 + 1) * 256, (tx1 - tx0 + 1) * 256))
for j, ty in enumerate(range(ty0, ty1 + 1)):
    for i, tx in enumerate(range(tx0, tx1 + 1)):
        if 0 <= ty < n:
            mosaic[j * 256:(j + 1) * 256, i * 256:(i + 1) * 256] = tile(Z, tx % n, ty)
elev = mosaic[Y0 - ty0 * 256:Y0 - ty0 * 256 + H, X0 - tx0 * 256:X0 - tx0 * 256 + W]
cell = 156543.03 * math.cos(math.radians(LAT)) / 2 ** Z
gy, gx = np.gradient(elev, cell)
# Steeper at the wide zooms, where a pixel spans a kilometre or more.
slope = np.arctan(np.hypot(gx, gy) * 3.0 * 2 ** (max(0, 9 - Z) / 2))
aspect = np.arctan2(-gx, gy)
az, alt = math.radians(315), math.radians(45)
shade = np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect)
layers[SHADE] = (np.clip(shade, 0, 1) * 255).astype(np.uint8)

# Places: the biggest that fit, clear of the page's panels (the layer chip
# at the top, the timeline at the bottom) and of each other.
font = ImageFont.truetype(fontp, 15 * S)
small = ImageFont.truetype(fontp, 13 * S)
cand = []
for f in load('ne_10m_populated_places_simple'):
    if not f['geometry']:
        continue
    lon, lat = f['geometry']['coordinates'][:2]
    x, y = px(lon, lat)
    if 12 * S < x < (W - 120) * S and 64 * S < y < (H - 96) * S:
        cand.append((f['properties'].get('pop_max', 0) or 0, f['properties']['name'], x, y))
cand.sort(reverse=True)
text, halo = canvas(), canvas()
dt, dh = ImageDraw.Draw(text), ImageDraw.Draw(halo)
taken = []
for pop, name, x, y in cand:
    f = font if len(taken) < 3 else small
    box = dt.textbbox((x + 8 * S, y - 9 * S), name, font=f)
    box = (min(box[0], x - 5 * S), box[1] - 3 * S, box[2] + 3 * S, box[3] + 3 * S)
    if any(not (box[2] < b[0] or box[0] > b[2] or box[3] < b[1] or box[1] > b[3]) for b in taken):
        continue
    taken.append(box)
    r = (4 if len(taken) == 1 else 3) * S
    dh.ellipse((x - r - S, y - r - S, x + r + S, y + r + S), fill=255)
    dt.ellipse((x - r, y - r, x + r, y + r), fill=255)
    dh.text((x + 8 * S, y - 9 * S), name, font=f, fill=255, stroke_width=2 * S, stroke_fill=255)
    dt.text((x + 8 * S, y - 9 * S), name, font=f, fill=255)
    if len(taken) == 8:
        break
layers[HALO] = down(halo)
layers[TEXT] = down(text)

# The file (version 2, R184 step 3): header, each layer's stream length,
# then one zlib stream a layer (the device unpacks a layer a pass), each row
# as differences from its left.
diff = layers.copy()
diff[:, :, 1:] = (layers[:, :, 1:].astype(np.int16) - layers[:, :, :-1]).astype(np.uint8)
streams = [zlib.compress(diff[k].tobytes(), 9) for k in range(8)]
head = b'RPML' + bytes([2]) + W.to_bytes(2, 'little') + H.to_bytes(2, 'little') + bytes([8]) + bytes(range(8))
head += b''.join(len(z).to_bytes(4, 'little') for z in streams)
out.write_bytes(head + b''.join(streams))
print(f'{out}: {out.stat().st_size} bytes, {len(taken)} places')
