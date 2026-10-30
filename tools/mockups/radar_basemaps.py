"""Draws three radar base-map looks for the Seattle area from Natural Earth and
open elevation tiles (args: maps dir, font file, out dir)."""
import json, math, sys, pathlib
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter

maps, fontp, out = map(pathlib.Path, sys.argv[1:4])
S = 2                     # draw at 2x, like the mock-up screenshots
Z = 9                     # web mercator zoom: 480 px ~ 100 km across
W, H = 480 * S, 640 * S
WORLD = 256 * 2 ** Z

def merc(lon, lat):
    x = (lon + 180) / 360 * WORLD
    r = math.radians(lat)
    y = (1 - math.log(math.tan(r) + 1 / math.cos(r)) / math.pi) / 2 * WORLD
    return x, y

CX, CY = merc(-122.40, 47.55)
X0, Y0 = CX - 240, CY - 320
def px(lon, lat):
    x, y = merc(lon, lat)
    return ((x - X0) * S, (y - Y0) * S)

def lonlat(x, y):
    lon = x / WORLD * 360 - 180
    lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y / WORLD))))
    return lon, lat
LON0, LAT1 = lonlat(X0 - 40, Y0 - 40)
LON1, LAT0 = lonlat(X0 + 520, Y0 + 680)

def inbox(coords):
    xs = [c[0] for c in coords]; ys = [c[1] for c in coords]
    return not (max(xs) < LON0 or min(xs) > LON1 or max(ys) < LAT0 or min(ys) > LAT1)

def load(name):
    return json.load(open(maps / 'ne' / (name + '.geojson'), encoding='utf-8'))['features']

def rings(geom):
    if not geom: return []
    t, c = geom['type'], geom['coordinates']
    if t == 'Polygon': return [c]
    if t == 'MultiPolygon': return c
    return []

def lines(geom):
    if not geom: return []
    t, c = geom['type'], geom['coordinates']
    if t == 'LineString': return [c]
    if t == 'MultiLineString': return c
    return []

land = [p for f in load('ne_10m_land') + load('ne_10m_minor_islands') for p in rings(f['geometry']) if inbox(p[0])]
lakes = [p for f in load('ne_10m_lakes') for p in rings(f['geometry']) if inbox(p[0])]
rivers = [l for f in load('ne_10m_rivers_lake_centerlines') for l in lines(f['geometry']) if inbox(l)]
roads = [(f['properties'].get('type', ''), l) for f in load('ne_10m_roads') for l in lines(f['geometry']) if inbox(l)]
states = [l for f in load('ne_10m_admin_1_states_provinces_lines') for l in lines(f['geometry']) if inbox(l)]
places = []
for f in load('ne_10m_populated_places_simple'):
    if not f['geometry']: continue
    lon, lat = f['geometry']['coordinates'][:2]
    if not (LON0 < lon < LON1 and LAT0 < lat < LAT1): continue
    x, y = px(lon, lat)
    if 30 < x < W - 140 and 120 < y < H - 160:
        places.append((f['properties'].get('pop_max', 0), f['properties']['name'], x, y))
places = sorted(places, reverse=True)[:7]

# elevation from terrarium tiles -> hillshade on the map's pixel grid
tx0, ty0 = int(X0 // 256), int(Y0 // 256)
mosaic = np.zeros((4 * 256, 4 * 256))
for i in range(4):
    for j in range(4):
        p = maps / 'terrain' / f'9_{tx0 + i}_{ty0 + j}.png'
        a = np.asarray(Image.open(p).convert('RGB'), dtype=np.float64)
        mosaic[j*256:(j+1)*256, i*256:(i+1)*256] = a[..., 0] * 256 + a[..., 1] + a[..., 2] / 256 - 32768
ox, oy = int(X0 - tx0 * 256), int(Y0 - ty0 * 256)
elev = mosaic[oy:oy + 640, ox:ox + 480]
elev = np.asarray(Image.fromarray(elev.astype(np.float32)).resize((W, H), Image.BICUBIC))
cell = 156543.03 * math.cos(math.radians(47.55)) / 2 ** Z / S
gy, gx = np.gradient(elev, cell)
slope = np.arctan(np.hypot(gx, gy) * 3.0)
aspect = np.arctan2(-gx, gy)
az, alt = math.radians(315), math.radians(45)
shade = np.sin(alt) * np.cos(slope) + np.cos(alt) * np.sin(slope) * np.cos(az - aspect)
shade = np.clip(shade, 0, 1)

def mask(polys, holes=True):
    m = Image.new('L', (W, H), 0); d = ImageDraw.Draw(m)
    for p in polys:
        d.polygon([px(*c[:2]) for c in p[0]], fill=255)
        if holes:
            for h in p[1:]:
                d.polygon([px(*c[:2]) for c in h], fill=0)
    return np.asarray(m) > 127
landm = mask(land) & ~mask(lakes)

# a radar-like rain field, the same for every look
rng = np.random.default_rng(7)
yy, xx = np.mgrid[0:H, 0:W] / S
field = np.zeros((H, W))
for cx, cy, sx, sy, a in [(120, 230, 120, 70, 1.0), (150, 250, 40, 22, .9), (380, 160, 90, 45, .7), (90, 460, 80, 45, .6), (300, 90, 50, 30, .5)]:
    field += a * np.exp(-(((xx - cx) / sx) ** 2 + ((yy - cy) / sy) ** 2))
noise = np.asarray(Image.fromarray((rng.random((H // 16, W // 16)) * 255).astype(np.uint8)).resize((W, H), Image.BICUBIC)) / 255
field = field * (0.7 + 0.6 * noise)
steps = [(0.25, (60, 190, 80)), (0.45, (30, 150, 60)), (0.7, (235, 215, 60)), (0.95, (240, 140, 40)), (1.15, (220, 50, 50))]
rain = np.zeros((H, W, 4), np.uint8)
for t, c in steps:
    sel = field >= t
    rain[sel, :3] = c; rain[sel, 3] = 175
rain_img = Image.fromarray(rain, 'RGBA')

font = ImageFont.truetype(str(fontp), 15 * S)
small = ImageFont.truetype(str(fontp), 13 * S)

def finish(base, line_col, road_col, river_col, state_col, text_col, halo_col, name):
    d = ImageDraw.Draw(base, 'RGBA')
    for l in rivers:
        d.line([px(*c[:2]) for c in l], fill=river_col, width=1 * S)
    for t, l in roads:
        major = 'Major' in t
        d.line([px(*c[:2]) for c in l], fill=road_col if major else road_col[:3] + (road_col[3] // 2,), width=(2 if major else 1) * S // (1 if major else 1))
    for l in states:
        pts = [px(*c[:2]) for c in l]
        for k in range(0, len(pts) - 1, 2):
            d.line(pts[k:k + 2], fill=state_col, width=S)
    # coastline: the land mask's edge
    edge = Image.fromarray((landm * 255).astype(np.uint8)).filter(ImageFilter.FIND_EDGES)
    base.paste(Image.new('RGBA', (W, H), line_col), (0, 0), edge.point(lambda v: 255 if v > 0 else 0))
    base.alpha_composite(rain_img)
    d = ImageDraw.Draw(base, 'RGBA')
    for k, (_, n, x, y) in enumerate(places):
        f = font if k < 3 else small
        d.ellipse((x - 4 * S, y - 4 * S, x + 4 * S, y + 4 * S) if k == 0 else (x - 3 * S, y - 3 * S, x + 3 * S, y + 3 * S), fill=text_col)
        d.text((x + 8 * S, y - 9 * S), n, font=f, fill=text_col, stroke_width=2 * S, stroke_fill=halo_col)
    base.convert('RGB').save(out / f'radar-{name}.png')

def rgb(a): return np.asarray(a, np.float64)

# 1: night glass, a dark map that sits under the smoked glass
water, ground = rgb((6, 14, 24)), rgb((34, 40, 46))
img = np.where(landm[..., None], ground * (0.55 + 0.9 * shade[..., None]), water)
finish(Image.fromarray(img.clip(0, 255).astype(np.uint8)).convert('RGBA'),
       (120, 180, 215, 220), (255, 255, 255, 38), (40, 80, 110, 255), (255, 255, 255, 70), (240, 242, 244, 255), (0, 0, 0, 200), 'night')

# 2: toner-style black and white
img = np.where(landm[..., None], rgb((236, 236, 232)), rgb((24, 24, 24)))
finish(Image.fromarray(img.astype(np.uint8)).convert('RGBA'),
       (0, 0, 0, 255), (0, 0, 0, 170), (24, 24, 24, 255), (0, 0, 0, 120), (10, 10, 10, 255), (255, 255, 255, 230), 'toner')

# 3: relief, tinted by height and shaded
e = np.clip(elev / 2200, 0, 1)[..., None]
tint = (1 - e) * rgb((92, 120, 84)) + e * rgb((190, 180, 160))
tint = np.where(elev[..., None] > 1900, rgb((235, 238, 240)), tint)
img = np.where(landm[..., None], tint * (0.25 + 1.0 * shade[..., None]), rgb((22, 50, 78)))
finish(Image.fromarray(img.clip(0, 255).astype(np.uint8)).convert('RGBA'),
       (200, 225, 240, 140), (255, 255, 255, 70), (60, 110, 150, 255), (255, 255, 255, 90), (255, 255, 255, 255), (0, 0, 0, 190), 'relief')
print('places', [p[1] for p in places], 'land', len(land), 'roads', len(roads))
