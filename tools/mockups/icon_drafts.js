// RetroPane icon drafts in the Crystal / Vista glass spirit: 256x256 masters.
// Each entry is SVG content for a 256 viewBox; ids are prefixed per icon.
// Glass recipe: a translucent body, a dark outline, a white inner rim, a
// strong gloss over the top, a soft light along the bottom edge.
const lin = (id, stops, x2 = 0, y2 = 1, x1 = 0, y1 = 0) =>
  `<linearGradient id="${id}" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}">${stops.map(([o, c, a]) => `<stop offset="${o}" stop-color="${c}"${a !== undefined ? ` stop-opacity="${a}"` : ''}/>`).join('')}</linearGradient>`;
const rad = (id, stops, cx = .5, cy = .5, r = .5, fx, fy) =>
  `<radialGradient id="${id}" cx="${cx}" cy="${cy}" r="${r}"${fx !== undefined ? ` fx="${fx}" fy="${fy}"` : ''}>${stops.map(([o, c, a]) => `<stop offset="${o}" stop-color="${c}"${a !== undefined ? ` stop-opacity="${a}"` : ''}/>`).join('')}</radialGradient>`;
const shadow = (cx, cy, rx, ry, id) => `<defs>${rad(id, [[0, '#000', .45], [1, '#000', 0]])}</defs><ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="url(#${id})"/>`;
const shine = (d, id, o = .85) => `<defs>${lin(id, [[0, '#fff', o], [.6, '#fff', o * .25], [1, '#fff', 0]])}</defs><path d="${d}" fill="url(#${id})"/>`;
const glow = (d, id, o = .5) => `<defs>${lin(id, [[0, '#fff', 0], [1, '#fff', o]])}</defs><path d="${d}" fill="url(#${id})"/>`;
const rim = (d, w = 3, o = .7) => `<path d="${d}" fill="none" stroke="#fff" stroke-opacity="${o}" stroke-width="${w}"/>`;
const streak = (x, y, h, r = -18) => `<rect x="${x}" y="${y}" width="10" height="${h}" rx="5" fill="#fff" opacity=".55" transform="rotate(${r} ${x} ${y})"/>`;

var ICONS = {};

// Shared parts for the full set (2026-10-30 batches).
// Colour ramps: light, mid, dark (the dark one also strokes the outline).
const C = {
  red: ['#ffc2b8', '#e3321e', '#8a1004'], blue: ['#c6ecff', '#2a7fe0', '#0b3a80'],
  green: ['#d6fab8', '#36b02a', '#126810'], yellow: ['#fff6c0', '#f2c01c', '#9a6a04'],
  orange: ['#ffe0b0', '#f58020', '#a03c02'], purple: ['#ecd0ff', '#9a4ae0', '#4a1480'],
  teal: ['#c9f4ee', '#3fb3a4', '#11574e'], silver: ['#ffffff', '#a9b4c0', '#4c5664'],
  graphite: ['#9aa6b6', '#3c4450', '#14181e'],
};
// Places another icon (its 256 box) centred on cx, cy at scale s: placing by centre
// keeps a nested drawing inside what holds it, where corner offsets drifted.
const place = (svg, cx, cy, s) => `<g transform="translate(${cx} ${cy}) scale(${s}) translate(-128 -128)">${svg}</g>`;
// A glossy ball: radial body, outline, highlight cap.
const orb = (id, cx, cy, r, [c0, c1, c2] = C.blue, o = .6) => `<defs>${rad(id, [[0, c0], [.55, c1], [1, c2]], .38, .3, .75)}</defs><circle cx="${cx}" cy="${cy}" r="${r}" fill="url(#${id})" stroke="${c2}" stroke-width="${Math.max(1.5, r / 16).toFixed(1)}"/><ellipse cx="${cx - r * .14}" cy="${cy - r * .46}" rx="${r * .62}" ry="${r * .34}" fill="#fff" opacity="${o}"/>`;
// A glossy rounded box (keys, tiles, buttons).
const key = (id, x, y, w, h, [c0, c1, c2] = C.silver, rx = 6) => (x = +x, y = +y, w = +w, h = +h, `<defs>${lin(id, [[0, c0], [.5, c1], [1, c2]])}</defs><rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${rx}" fill="url(#${id})" stroke="${c2}" stroke-width="1.5"/><rect x="${x + 3}" y="${y + 2}" width="${w - 6}" height="${h * .42}" rx="${rx * .7}" fill="#fff" opacity=".5"/>`);
// A sheet of paper with a folded corner; inner is drawn on the page.
const page = (p, inner = '', x = 52, y = 22, w = 152, h = 208) => {
  const f = 40;
  return `${shadow(x + w / 2, y + h + 6, w * .62, 12, p + '-sh')}
<defs>${lin(p + '-pg', [[0, '#ffffff'], [.7, '#f0f4f8'], [1, '#cfd9e4']], 1, 1)}${lin(p + '-fd', [[0, '#ffffff'], [1, '#b6c3d2']], 1, 1)}</defs>
<path d="M${x} ${y + 6} Q${x} ${y} ${x + 6} ${y} L${x + w - f} ${y} L${x + w} ${y + f} L${x + w} ${y + h - 6} Q${x + w} ${y + h} ${x + w - 6} ${y + h} L${x + 6} ${y + h} Q${x} ${y + h} ${x} ${y + h - 6} Z" fill="url(#${p}-pg)" stroke="#7d8896" stroke-width="2.5"/>
${inner}
<path d="M${x + w - f} ${y} L${x + w - f} ${y + f - 6} Q${x + w - f} ${y + f} ${x + w - f + 6} ${y + f} L${x + w} ${y + f} Z" fill="url(#${p}-fd)" stroke="#7d8896" stroke-width="2" stroke-linejoin="round"/>
${shine(`M${x + 3} ${y + 3} L${x + w - f} ${y + 3} L${x + w - f + 12} ${y + 44} Q${x + w / 2} ${y + 76} ${x + 3} ${y + 66} Z`, p + '-gl', .5)}`;
};
// Grey text lines on a page.
const lines = (x, y, w, n, gap = 18, c = '#b4bdc8') => `<g fill="${c}">${[...Array(n)].map((_, i) => `<rect x="${x}" y="${y + i * gap}" width="${i % 3 == 2 ? w * .7 : w}" height="8" rx="4"/>`).join('')}</g>`;
// A magnifying glass: lens centre, radius; the handle runs to the lower right.
const lens = (p, cx, cy, r) => `<defs>${rad(p + '-ln', [[0, '#ffffff', .9], [.55, '#9fd8ff', .5], [1, '#2a7fe0', .8]], .38, .32, .75)}${lin(p + '-rm', [[0, '#ffffff'], [.5, '#a6b0bc'], [1, '#4c5662']], 1, 1)}${lin(p + '-hd', [[0, '#6a7480'], [.5, '#2c323c'], [1, '#0e1116']], 1, 0)}</defs>
<g transform="rotate(-45 ${cx} ${cy})"><rect x="${cx - r * .2}" y="${cy + r * .95}" width="${r * .4}" height="${r * 1.25}" rx="${r * .18}" fill="url(#${p}-hd)" stroke="#05060a" stroke-width="2"/><rect x="${cx - r * .13}" y="${cy + r}" width="${r * .1}" height="${r * 1.1}" rx="${r * .05}" fill="#fff" opacity=".4"/></g>
<circle cx="${cx}" cy="${cy}" r="${r}" fill="url(#${p}-ln)" stroke="url(#${p}-rm)" stroke-width="${r * .2}"/>
<circle cx="${cx}" cy="${cy}" r="${r * 1.1}" fill="none" stroke="#3a4250" stroke-width="2"/><circle cx="${cx}" cy="${cy}" r="${r * .9}" fill="none" stroke="#3a4250" stroke-width="1.5"/>
<ellipse cx="${cx - r * .25}" cy="${cy - r * .38}" rx="${r * .5}" ry="${r * .26}" fill="#fff" opacity=".7" transform="rotate(-30 ${cx - r * .25} ${cy - r * .38})"/>`;
// A curved arrow along a circle (centre cx, cy, radius r) from angle a0 to a1 (degrees, clockwise).
const arcArrow = (cx, cy, r, a0, a1, stroke, w = 20, head = 1.1) => {
  const P = (a, rr = r) => [cx + rr * Math.cos(a * Math.PI / 180), cy + rr * Math.sin(a * Math.PI / 180)];
  const [x0, y0] = P(a0), [x1, y1] = P(a1), t = a1 * Math.PI / 180, tx = -Math.sin(t), ty = Math.cos(t);
  const tip = [x1 + tx * w * head, y1 + ty * w * head], b1 = P(a1, r + w), b2 = P(a1, r - w);
  const big = Math.abs(a1 - a0) > 180 ? 1 : 0;
  return `<path d="M${x0.toFixed(1)} ${y0.toFixed(1)} A${r} ${r} 0 ${big} 1 ${x1.toFixed(1)} ${y1.toFixed(1)}" fill="none" stroke="${stroke}" stroke-width="${w}"/>
<path d="M${b1[0].toFixed(1)} ${b1[1].toFixed(1)} L${tip[0].toFixed(1)} ${tip[1].toFixed(1)} L${b2[0].toFixed(1)} ${b2[1].toFixed(1)} Z" fill="${stroke}" stroke-linejoin="round"/>`;
};

ICONS.Calendar = `
${shadow(128, 232, 100, 14, 'cal-sh')}
<defs>
${lin('cal-pg', [[0, '#ffffff'], [.7, '#f2f6fa'], [1, '#d4dde7']])}
${lin('cal-bk', [[0, '#e6ecf2'], [1, '#a9b6c4']])}
${lin('cal-hd', [[0, '#ff9c80'], [.45, '#e8402a'], [1, '#a0180a']])}
${lin('cal-ring', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#5d6874']], 1, 0)}
</defs>
<rect x="46" y="48" width="176" height="178" rx="10" fill="url(#cal-bk)" stroke="#6d7886" stroke-width="2"/>
<rect x="40" y="42" width="176" height="178" rx="10" fill="#eef2f6" stroke="#7d8896" stroke-width="2"/>
<rect x="34" y="36" width="176" height="178" rx="10" fill="url(#cal-pg)" stroke="#6d7886" stroke-width="2.5"/>
<path d="M34 46 Q34 36 44 36 L200 36 Q210 36 210 46 L210 90 L34 90 Z" fill="url(#cal-hd)" stroke="#7a1008" stroke-width="2.5"/>
${shine('M37 39 L207 39 L207 64 Q122 74 37 64 Z', 'cal-gl', .9)}
<text x="122" y="146" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="54" fill="#2b3644">30</text>
<g fill="#c6ced8">${[0, 1, 2, 3, 4, 5].map(i => [0, 1, 2].map(j => `<rect x="${52 + i * 24}" y="${158 + j * 15}" width="17" height="9" rx="2"/>`).join('')).join('')}</g>
<rect x="124" y="173" width="17" height="9" rx="2" fill="#e8402a"/>
${shine('M36 92 L208 92 L208 120 Q122 104 36 126 Z', 'cal-pgl', .5)}
<g>${[66, 98, 146, 178].map(x => `<rect x="${x - 6}" y="20" width="12" height="36" rx="6" fill="url(#cal-ring)" stroke="#4a5460"/>`).join('')}</g>`;

ICONS.Tasks = `
${shadow(128, 232, 94, 14, 'tk-sh')}
<defs>
${lin('tk-bd', [[0, '#8fd0ff', .82], [.5, '#2f7fe0', .72], [1, '#0d3f86', .9]], 1, 1)}
${lin('tk-pp', [[0, '#ffffff'], [1, '#e2e9f1']])}
${lin('tk-cl', [[0, '#ffffff'], [.5, '#a6b0bc'], [1, '#5c6672']])}
${lin('tk-ck', [[0, '#b6f78e'], [.5, '#33b02a'], [1, '#13751a']])}
</defs>
<rect x="44" y="34" width="168" height="196" rx="16" fill="url(#tk-bd)" stroke="#0a2f66" stroke-width="3"/>
${rim('M60 37 H196 Q209 37 209 50 V214 Q209 227 196 227 H60 Q47 227 47 214 V50 Q47 37 60 37 Z', 2.5, .75)}
<rect x="62" y="58" width="132" height="156" rx="4" fill="url(#tk-pp)" stroke="#9aa6b4" stroke-width="2"/>
<rect x="92" y="20" width="72" height="40" rx="10" fill="url(#tk-cl)" stroke="#47515c" stroke-width="3"/>
${shine('M95 23 H161 V38 Q128 44 95 38 Z', 'tk-clg', .9)}
<circle cx="128" cy="34" r="7" fill="#47515c"/>
${[92, 134, 176].map((y, i) => `<rect x="76" y="${y - 14}" width="26" height="26" rx="5" fill="#fff" stroke="#8592a2" stroke-width="2"/>
${i < 2 ? `<path d="M78 ${y - 2} l9 10 l20 -26" fill="none" stroke="url(#tk-ck)" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>` : ''}
<rect x="114" y="${y - 5}" width="66" height="9" rx="4" fill="${i < 2 ? '#b4bdc8' : '#7f8a98'}"/>`).join('')}
${shine('M46 36 L210 36 L210 96 Q128 124 46 104 Z', 'tk-gl', .55)}
${glow('M46 200 L210 200 L210 228 L46 228 Z', 'tk-bg', .45)}`;

ICONS.Notes = `
${shadow(124, 232, 98, 14, 'nt-sh')}
<defs>
${lin('nt-pp', [[0, '#fffbc8'], [.6, '#ffe45a'], [1, '#e9b81c']])}
${lin('nt-sp', [[0, '#ffffff'], [.5, '#9aa4b0'], [1, '#4c5662']], 1, 0)}
${lin('nt-pc', [[0, '#ffe0a0'], [.45, '#f28b1c'], [1, '#a84a08']], 1, 0)}
</defs>
<path d="M46 44 L196 44 Q206 44 206 54 L206 210 L46 210 Z" fill="#d79f12"/>
<path d="M40 36 L190 36 Q200 36 200 46 L200 202 Q200 208 194 208 L46 208 Q40 208 40 202 Z" fill="url(#nt-pp)" stroke="#a87a0a" stroke-width="2"/>
<g stroke="#d7a52a" stroke-width="3">${[86, 112, 138, 164, 188].map(y => `<path d="M58 ${y} H184"/>`).join('')}</g>
${shine('M42 38 L198 38 L198 100 Q120 124 42 96 Z', 'nt-gl', .7)}
<g>${[60, 88, 116, 144, 172].map(x => `<rect x="${x}" y="24" width="10" height="28" rx="5" fill="url(#nt-sp)" stroke="#3e4650"/>`).join('')}</g>
<g transform="rotate(38 172 160)"><rect x="160" y="70" width="28" height="140" rx="5" fill="url(#nt-pc)" stroke="#7a3404" stroke-width="2"/>
<rect x="160" y="70" width="28" height="22" rx="4" fill="#f6b0c2" stroke="#a04a60" stroke-width="2"/><rect x="160" y="92" width="28" height="10" fill="#dfe5ec" stroke="#6a737e"/>
<path d="M160 210 L174 244 L188 210 Z" fill="#f4d9a8" stroke="#8a6a3a" stroke-width="2"/><path d="M169 232 L174 244 L179 232 Z" fill="#333"/>
<rect x="164" y="74" width="7" height="134" rx="3" fill="#fff" opacity=".6"/></g>`;

ICONS.Inbox = `
${shadow(128, 224, 106, 14, 'ib-sh')}
<defs>
${lin('ib-bd', [[0, '#ffffff', .92], [.6, '#e1ecf5', .8], [1, '#a9c2da', .9]])}
${lin('ib-fl', [[0, '#bfe6ff', .9], [.5, '#4aa0f0', .78], [1, '#1a59b0', .92]])}
${rad('ib-bg', [[0, '#ffb0a2'], [.55, '#e3321e'], [1, '#9a1206']], .4, .35, .7)}
</defs>
<path d="M28 76 L228 76 L228 200 Q228 208 220 208 L36 208 Q28 208 28 200 Z" fill="url(#ib-bd)" stroke="#5b6c80" stroke-width="3"/>
<path d="M28 200 L108 132 M228 200 L148 132" stroke="#8fa4bc" stroke-width="3"/>
${glow('M30 176 L226 176 L226 206 L30 206 Z', 'ib-gw', .7)}
<path d="M28 76 L128 154 L228 76 Z" fill="url(#ib-fl)" stroke="#123f80" stroke-width="3" stroke-linejoin="round"/>
${rim('M38 80 L128 148 L218 80', 2.5, .8)}
${shine('M34 80 L222 80 L128 150 Z', 'ib-gl', .8)}
${streak(48, 92, 92, -4)}
<circle cx="204" cy="70" r="30" fill="url(#ib-bg)" stroke="#fff" stroke-width="6"/>
<ellipse cx="200" cy="58" rx="18" ry="9" fill="#fff" opacity=".55"/>
<text x="204" y="83" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="36" fill="#fff">3</text>`;

const folder = (p, inner = '') => `
${shadow(128, 226, 110, 14, p + '-sh')}
<defs>
${lin(p + '-bk', [[0, '#a8ece2', .95], [.5, '#3fb3a4', .9], [1, '#18786c', .95]])}
${lin(p + '-fr', [[0, '#d6f8f3', .7], [.45, '#6fd0c2', .55], [1, '#1d8a7c', .85]])}
</defs>
<path d="M30 52 Q30 42 40 42 L100 42 Q108 42 112 50 L120 62 L214 62 Q224 62 224 72 L224 206 L30 206 Z" fill="url(#${p}-bk)" stroke="#11574e" stroke-width="3"/>
${rim('M36 52 Q36 48 42 48 L98 48 Q104 48 107 54 L114 68 L212 68 Q218 68 218 74', 2.5, .8)}
${inner}
<path d="M22 100 Q22 92 30 92 L226 92 Q234 92 233 100 L222 208 Q221 216 213 216 L40 216 Q32 216 31 208 Z" fill="url(#${p}-fr)" stroke="#11574e" stroke-width="3"/>
${rim('M28 98 L228 98 L218 208 L38 208 Z', 2.5, .75)}
${shine('M26 96 L230 96 L227 128 Q128 144 28 128 Z', p + '-gl', .9)}
${glow('M34 186 L222 186 L220 210 L38 210 Z', p + '-gw', .5)}
${streak(196, 104, 92, 6)}`;

ICONS.Folder = folder('fo', `<rect x="56" y="54" width="132" height="70" rx="3" fill="#fff" stroke="#9aa6b4" stroke-width="2" transform="rotate(-4 120 90)"/>`);

ICONS.Music = folder('mu', `
<defs>${lin('mu-nt', [[0, '#a8dcff'], [.5, '#2a7fe0'], [1, '#0c3c88']])}</defs>
<path d="M118 56 L190 38 L190 150 A22 17 0 1 1 172 134 L172 72 L136 81 L136 168 A22 17 0 1 1 118 152 Z" fill="url(#mu-nt)" stroke="#0a2a60" stroke-width="3"/>
<path d="M124 62 L184 47 L184 58 L124 73 Z" fill="#fff" opacity=".7"/>`);

ICONS.Browser = `
${shadow(128, 228, 94, 14, 'br-sh')}
<defs>
${rad('br-gl', [[0, '#e6f6ff'], [.3, '#6cbcff'], [.78, '#1257b8'], [1, '#062c6a']], .38, .32, .75)}
${lin('br-ld', [[0, '#c4f59a'], [.5, '#3fae2c'], [1, '#1a6a14']])}
${lin('br-rf', [[0, '#ffe7a0'], [.5, '#f5a51e'], [1, '#b05e04']], 0, 1)}
${lin('br-rb', [[0, '#c98a24'], [1, '#7a4304']], 0, 1)}
${rad('br-rm', [[.82, '#fff', 0], [.96, '#fff', .55], [1, '#fff', 0]])}
</defs>
<g transform="rotate(-18 128 122)"><path d="M8 122 A120 34 0 0 1 248 122" fill="none" stroke="url(#br-rb)" stroke-width="11" stroke-linecap="round"/></g>
<circle cx="128" cy="120" r="88" fill="url(#br-gl)" stroke="#05285e" stroke-width="3"/>
<path d="M74 70 Q98 52 120 66 Q130 82 112 94 Q100 104 108 122 Q116 138 96 152 Q80 156 72 138 Q62 120 56 102 Q58 82 74 70 Z M150 54 Q174 58 192 82 Q200 104 186 110 Q170 108 160 96 Q150 82 142 72 Q142 58 150 54 Z M148 150 Q168 140 186 150 Q182 172 160 190 Q148 188 148 172 Z" fill="url(#br-ld)" stroke="#145a10" stroke-width="2" opacity=".95"/>
<circle cx="128" cy="120" r="88" fill="url(#br-rm)"/>
<ellipse cx="104" cy="74" rx="56" ry="30" fill="#fff" opacity=".55" transform="rotate(-20 104 74)"/>
<ellipse cx="150" cy="182" rx="40" ry="12" fill="#fff" opacity=".22" transform="rotate(-20 150 182)"/>
<g transform="rotate(-18 128 122)"><path d="M8 122 A120 34 0 0 0 248 122" fill="none" stroke="url(#br-rf)" stroke-width="13" stroke-linecap="round"/>
<path d="M30 136 A110 26 0 0 0 220 138" fill="none" stroke="#fff" stroke-width="3" opacity=".7"/></g>`;

ICONS.Media = `
${shadow(128, 228, 98, 14, 'md-sh')}
<defs>
${rad('md-dk', [[0, '#ffffff', .95], [.3, '#e8eef6', .85], [.55, '#b8c8da', .8], [.78, '#eef3f8', .9], [1, '#8592a2', .95]], .45, .42, .62)}
${rad('md-pl', [[0, '#ffe0b0', .95], [.5, '#f58020', .85], [1, '#a03c02', .95]], .4, .32, .78)}
</defs>
<circle cx="112" cy="124" r="88" fill="url(#md-dk)" stroke="#55616e" stroke-width="3"/>
<path d="M58 72 A80 80 0 0 1 150 50" stroke="#ff9ad8" stroke-width="12" fill="none" opacity=".45"/>
<path d="M52 112 A64 64 0 0 1 96 64" stroke="#8ae0ff" stroke-width="12" fill="none" opacity=".45"/>
<path d="M160 190 A64 64 0 0 1 120 200" stroke="#b6ff9a" stroke-width="10" fill="none" opacity=".4"/>
<circle cx="112" cy="124" r="22" fill="#f4f7fa" stroke="#6e7a88" stroke-width="3"/><circle cx="112" cy="124" r="8" fill="#5c6672"/>
${streak(70, 60, 120, -40)}
<circle cx="182" cy="168" r="52" fill="url(#md-pl)" stroke="#7a2c02" stroke-width="3"/>
${rim('M182 120 A48 48 0 1 1 181.9 120', 2.5, .6)}
<path d="M168 144 L208 168 L168 192 Z" fill="#fff"/>
<ellipse cx="174" cy="142" rx="34" ry="17" fill="#fff" opacity=".6"/>`;

ICONS.Games = `
${shadow(128, 218, 114, 14, 'gm-sh')}
<defs>
${lin('gm-bd', [[0, '#7c8696', .95], [.35, '#2c323c', .92], [1, '#0e1116']])}
${rad('gm-a', [[0, '#ffc2b8'], [.6, '#e3321e'], [1, '#8a1004']], .4, .3, .7)}
${rad('gm-b', [[0, '#c6ecff'], [.6, '#2a7fe0'], [1, '#0b3a80']], .4, .3, .7)}
${rad('gm-c', [[0, '#d6fab8'], [.6, '#36b02a'], [1, '#126810']], .4, .3, .7)}
${rad('gm-d', [[0, '#fff6c0'], [.6, '#f2c01c'], [1, '#9a6a04']], .4, .3, .7)}
</defs>
<path d="M58 78 Q128 60 198 78 Q232 86 238 140 Q244 196 210 200 Q190 202 170 168 L86 168 Q66 202 46 200 Q12 196 18 140 Q24 86 58 78 Z" fill="url(#gm-bd)" stroke="#050608" stroke-width="3"/>
${rim('M60 84 Q128 66 196 84 Q226 92 232 140', 2.5, .55)}
${shine('M60 82 Q128 64 196 82 Q218 90 226 116 Q128 100 30 116 Q38 90 60 82 Z', 'gm-gl', .6)}
<path d="M62 120 h16 v-16 h16 v16 h16 v16 h-16 v16 h-16 v-16 h-16 z" fill="#d6dce4" stroke="#3a4048" stroke-width="2"/>
${[['c', 184, 106], ['a', 208, 128], ['b', 160, 128], ['d', 184, 150]].map(([k, x, y]) => `<circle cx="${x}" cy="${y}" r="13" fill="url(#gm-${k})" stroke="#111" stroke-width="1.5"/><ellipse cx="${x - 3}" cy="${y - 5}" rx="7" ry="4" fill="#fff" opacity=".7"/>`).join('')}
${glow('M40 170 Q60 196 46 198 Q22 196 22 160 Z', 'gm-gw', .35)}`;

ICONS.Settings = `
${shadow(128, 228, 94, 14, 'st-sh')}
<defs>
${lin('st-gr', [[0, '#ffffff'], [.35, '#d4dbe3'], [.65, '#8592a2'], [1, '#4c5664']], 1, 1)}
${rad('st-cn', [[0, '#e0f4ff', .95], [.45, '#3a92ea', .85], [1, '#0b3a80', .95]], .4, .32, .72)}
</defs>
<g transform="translate(128 122)">
<path d="${(() => { let d = ''; const n = 10; for (let i = 0; i < n; i++) { const a = i / n * Math.PI * 2, b = a + Math.PI / n * .55, c = a + Math.PI / n * 1.45, e = a + Math.PI / n * 2; const P = (r, t) => `${(r * Math.cos(t)).toFixed(1)} ${(r * Math.sin(t)).toFixed(1)}`; d += (i ? 'L' : 'M') + P(78, a) + 'L' + P(96, b) + 'L' + P(96, c) + 'L' + P(78, e); } return d + 'Z'; })()}" fill="url(#st-gr)" stroke="#3a4250" stroke-width="3"/>
<circle r="72" fill="none" stroke="#fff" stroke-width="3" opacity=".6"/>
<circle r="48" fill="url(#st-cn)" stroke="#0a2a60" stroke-width="3"/>
<circle r="44" fill="none" stroke="#fff" stroke-width="2.5" opacity=".6"/>
<circle r="17" fill="#eef2f6" stroke="#3a4250" stroke-width="3"/>
<ellipse cx="-12" cy="-24" rx="28" ry="13" fill="#fff" opacity=".7"/>
<ellipse cx="10" cy="30" rx="20" ry="6" fill="#fff" opacity=".25"/></g>`;

ICONS['My Device'] = `
${shadow(128, 236, 72, 12, 'dv-sh')}
<defs>
${lin('dv-bd', [[0, '#9aa6b6'], [.15, '#3c4450'], [.85, '#1a1e24'], [1, '#4a5260']], 1, 0)}
${lin('dv-sc', [[0, '#cdeeff'], [.5, '#3c9cf0'], [1, '#0c4aa0']])}
${rad('dv-bt', [[0, '#f2f6fa'], [1, '#6c7886']], .4, .3, .7)}
</defs>
<rect x="66" y="18" width="124" height="208" rx="18" fill="url(#dv-bd)" stroke="#0a0c10" stroke-width="3"/>
${rim('M84 21 H172 Q187 21 187 36', 2.5, .6)}
<rect x="80" y="38" width="96" height="124" rx="3" fill="url(#dv-sc)" stroke="#05080c" stroke-width="2"/>
<path d="M80 128 Q128 100 176 118 L176 162 L80 162 Z" fill="#5fc46a" opacity=".6"/>
<circle cx="150" cy="64" r="12" fill="#ffe27a"/>
<path d="M80 38 L176 38 L176 72 L80 120 Z" fill="#fff" opacity=".42"/>
<circle cx="128" cy="194" r="17" fill="url(#dv-bt)" stroke="#2a3038" stroke-width="2"/>
<ellipse cx="124" cy="187" rx="9" ry="5" fill="#fff" opacity=".7"/>
${[92, 164].map(x => `<rect x="${x - 10}" y="186" width="20" height="14" rx="4" fill="#5c6672" stroke="#2a3038"/>`).join('')}
<rect x="70" y="22" width="10" height="200" rx="5" fill="#fff" opacity=".22"/>`;

// The trash can (the owner's name for it, 2026-10-30; WM called it Deleted Items).
const trash = (p, inner = '', over = '') => `
${shadow(128, 234, 86, 13, p + '-sh')}
<defs>
${lin(p + '-gl', [[0, '#e4fbf8', .75], [.5, '#76d0c4', .5], [1, '#2a9486', .82]], 1, 0)}
${lin(p + '-rm', [[0, '#ffffff', .95], [1, '#7fc9be', .9]])}
</defs>
${inner}
<path d="M50 52 L206 52 L188 220 Q186 228 178 228 L78 228 Q70 228 68 220 Z" fill="url(#${p}-gl)" stroke="#155e55" stroke-width="3"/>
${rim('M58 62 L198 62 L181 218 L75 218 Z', 2.5, .7)}
<ellipse cx="128" cy="52" rx="80" ry="15" fill="url(#${p}-rm)" stroke="#155e55" stroke-width="3"/>
<ellipse cx="128" cy="54" rx="67" ry="8" fill="#2a9486" opacity=".35"/>
${over}
<g transform="translate(0 4)">${[0, 120, 240].map(r => `<g transform="rotate(${r} 128 146)"><path d="M134.2 116.8 L153.6 150.4" stroke="#fff" stroke-width="10" stroke-linecap="round" opacity=".92"/><path d="M160.6 162.5 L164 144.4 L143.2 156.4 Z" fill="#fff" stroke="#fff" stroke-width="3" stroke-linejoin="round" opacity=".92"/></g>`).join('')}</g>
<path d="M62 70 L86 70 L96 214 L82 214 Z" fill="#fff" opacity=".45"/>
${glow('M72 198 L184 198 L182 220 L74 220 Z', p + '-gw', .5)}`;

ICONS['Trash can'] = trash('bn');
