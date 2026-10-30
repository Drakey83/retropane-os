"""Checks every icon draft (tools/mockups/icon_drafts*.js) for drawing faults.

Flags: content past the 256 px canvas, a centre far from the middle, NaN or
undefined in the SVG, and ids that two icons define differently (on one page
the first definition wins, so the second icon takes the wrong gradient).
Run: python tools/mockups/icon_audit.py   (exit 1 on any finding)
"""
import io, pathlib, re, sys, tempfile
from playwright.sync_api import sync_playwright

here = pathlib.Path(__file__).resolve().parent
js = '\n'.join(io.open(f, encoding='utf-8').read() for f in [here / 'icon_drafts.js'] + sorted(here.glob('icon_drafts_*.js')))
page = '<!doctype html><meta charset="utf-8"><body><script>' + js + '</script></body>'
# Icons drawn off-centre on purpose: a badge or a second object sits to one side.
OFFSET_OK = {'Inbox', 'Media', 'Today', 'Weather', 'File Explorer', 'Record', 'Remote Desktop', 'Search',
             'Badge: Magnifier', 'Magnifier', 'Clock & region', 'Owner info', 'Sources & accounts', 'About',
             'Wireless', 'Display & brightness', 'Sound', 'Security & keys', 'Packs', 'USB / PC',
             'Themes: wallpaper swap', 'Themes & desktop', 'Example: shortcut', 'Touch alignment'}

with tempfile.TemporaryDirectory() as d:
    f = pathlib.Path(d) / 'audit.html'
    f.write_text(page, encoding='utf-8')
    with sync_playwright() as p:
        b = p.chromium.launch(); pg = b.new_page()
        pg.goto(f.as_uri())
        res = pg.evaluate('''() => Object.entries(ICONS).map(([n, s]) => {
            const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
            svg.setAttribute('viewBox', '0 0 256 256'); svg.setAttribute('width', 256); svg.setAttribute('height', 256);
            svg.innerHTML = '<g id="probe">' + s + '</g>'; document.body.appendChild(svg);
            const bb = svg.querySelector('#probe').getBBox();
            const ids = [...svg.querySelectorAll('[id]')].filter(e => e.id != 'probe').map(e => [e.id, e.outerHTML]);
            svg.remove();
            return {n, x0: bb.x, y0: bb.y, x1: bb.x + bb.width, y1: bb.y + bb.height, bad: /NaN|undefined/.test(s), ids};
        })''')
        b.close()

found = []
defs = {}
for r in res:
    n = r['n']
    if r['bad']: found.append(f'{n}: NaN or undefined in the SVG')
    if r['x0'] < -2 or r['y0'] < -2 or r['x1'] > 258 or r['y1'] > 258:
        found.append(f"{n}: past the canvas ({r['x0']:.0f},{r['y0']:.0f})-({r['x1']:.0f},{r['y1']:.0f})")
    cx = (r['x0'] + r['x1']) / 2
    if abs(cx - 128) > 12 and n not in OFFSET_OK:
        found.append(f"{n}: centre {cx:.0f} (x {r['x0']:.0f}-{r['x1']:.0f}), not 128")
    for i, h in r['ids']:
        defs.setdefault(i, {}).setdefault(re.sub(r'\s+', ' ', h), set()).add(n)
for i, v in defs.items():
    if len(v) > 1:
        found.append(f"id {i}: defined {len(v)} ways, by " + '; '.join(', '.join(sorted(s)) for s in v.values()))
print('\n'.join(found) or 'no findings', f'({len(res)} icons)')
sys.exit(1 if found else 0)
