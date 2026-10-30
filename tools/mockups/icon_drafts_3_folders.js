// Icon drafts, batch 3: the teal glass folders (icon-brief.md "The full set").
// What a folder holds stands inside it; what it is for sits on its front.

const starPath = (cx, cy, R, r = R * .45) => 'M' + [...Array(10)].map((_, i) => { const a = -Math.PI / 2 + i * Math.PI / 5, q = i % 2 ? r : R; return `${(cx + q * Math.cos(a)).toFixed(1)} ${(cy + q * Math.sin(a)).toFixed(1)}`; }).join(' L') + ' Z';
const sheetIn = (x, y, r, w = 110, h = 120) => `<g transform="rotate(${r} ${x + w / 2} ${y + h / 2})"><rect x="${x}" y="${y}" width="${w}" height="${h}" rx="3" fill="#fbfdff" stroke="#8592a2" stroke-width="2"/>${lines(x + 12, y + 14, w - 24, 4, 14)}</g>`;

ICONS['Folder, open'] = `
${shadow(128, 228, 112, 14, 'fp-sh')}
<defs>
${lin('fp-bk', [[0, '#a8ece2', .95], [.5, '#3fb3a4', .9], [1, '#18786c', .95]])}
${lin('fp-fr', [[0, '#d6f8f3', .72], [.45, '#6fd0c2', .58], [1, '#1d8a7c', .88]])}
</defs>
<path d="M34 46 Q34 36 44 36 L100 36 Q108 36 112 44 L120 56 L210 56 Q220 56 220 66 L220 206 L34 206 Z" fill="url(#fp-bk)" stroke="#11574e" stroke-width="3"/>
${rim('M40 46 Q40 42 46 42 L98 42 Q104 42 107 48 L114 62 L208 62 Q214 62 214 68', 2.5, .8)}
${sheetIn(60, 30, -8)}${sheetIn(96, 40, 6)}
<path d="M6 128 Q4 118 14 118 L240 118 Q250 118 248 128 L224 210 Q222 218 214 218 L42 218 Q34 218 32 210 Z" fill="url(#fp-fr)" stroke="#11574e" stroke-width="3"/>
${rim('M14 124 L240 124 L218 210 L40 210 Z', 2.5, .75)}
${shine('M10 122 L244 122 L238 148 Q128 160 16 148 Z', 'fp-gl', .9)}
${glow('M36 190 L222 190 L216 212 L42 212 Z', 'fp-gw', .5)}
${streak(206, 130, 76, 12)}`;

ICONS['Documents folder'] = folder('fd', `${sheetIn(54, 40, -6)}${sheetIn(92, 46, 5)}`);

ICONS['Pictures folder'] = folder('fi', `
<defs>${lin('fi-sk', [[0, '#7cc6ff'], [1, '#d8f0ff']])}${lin('fi-hl', [[0, '#8fe07a'], [1, '#2a8a20']])}</defs>
<g transform="rotate(-6 128 90)"><rect x="56" y="34" width="140" height="100" rx="3" fill="#fbfdff" stroke="#7d8896" stroke-width="2"/>
<rect x="64" y="42" width="124" height="78" fill="url(#fi-sk)"/><circle cx="166" cy="60" r="10" fill="#ffe27a"/>
<path d="M64 108 Q90 76 116 98 Q146 72 188 100 L188 120 L64 120 Z" fill="url(#fi-hl)"/></g>`);

ICONS['Video folder'] = folder('fv', `
<defs>${lin('fv-cb', [[0, '#5a6474'], [1, '#14181e']])}</defs>
<g transform="rotate(-5 128 90)">
<rect x="62" y="58" width="132" height="80" rx="4" fill="url(#fv-cb)" stroke="#000" stroke-width="2"/>
<g transform="rotate(-14 62 52)"><rect x="62" y="36" width="132" height="22" rx="3" fill="#f4f6f8" stroke="#000" stroke-width="2"/>
${[0, 1, 2, 3].map(i => `<path d="M${74 + i * 32} 36 L${92 + i * 32} 36 L${80 + i * 32} 58 L${62 + i * 32} 58 Z" fill="#1c2026"/>`).join('')}</g>
<rect x="62" y="58" width="132" height="12" fill="#f4f6f8" stroke="#000" stroke-width="2"/>
${[0, 1, 2, 3].map(i => `<path d="M${74 + i * 32} 58 L${92 + i * 32} 58 L${84 + i * 32} 70 L${66 + i * 32} 70 Z" fill="#1c2026"/>`).join('')}
<rect x="66" y="74" width="124" height="14" rx="3" fill="#fff" opacity=".25"/></g>`);

// A badge on a folder's front, lower right.
const front = (body, cx = 180, cy = 178, r = 40) => `<circle cx="${cx}" cy="${cy}" r="${r + 5}" fill="#fff" opacity=".9"/>${body}`;

ICONS['Downloads folder'] = folder('fw', sheetIn(70, 42, -4)) + front(`
${orb('fw-o', 180, 178, 40, C.green, .45)}
<path d="M168 150 H192 V176 H206 L180 204 L154 176 H168 Z" fill="#fff" stroke="#fff" stroke-width="2" stroke-linejoin="round"/>`);

ICONS['Games folder'] = folder('fg', `${mini('Games', 50, 4, .6)}`);

ICONS['Programs folder'] = ICONS['Programs: folder'];

ICONS['Shared folder'] = folder('fs', sheetIn(70, 42, -4)) + front(`
${orb('fs-o', 180, 178, 40, C.blue, 0)}
<defs>${rad('fs-p', [[0, '#ffffff'], [1, '#cfe6ff']], .4, .3, .8)}</defs>
<circle cx="166" cy="166" r="11" fill="url(#fs-p)"/><path d="M148 200 Q148 180 166 180 Q184 180 184 200 Z" fill="url(#fs-p)"/>
<circle cx="194" cy="166" r="11" fill="url(#fs-p)" stroke="#2a7fe0" stroke-width="2"/><path d="M176 200 Q176 180 194 180 Q212 180 212 200 Z" fill="url(#fs-p)" stroke="#2a7fe0" stroke-width="2"/>
<ellipse cx="174" cy="150" rx="24" ry="10" fill="#fff" opacity=".4"/>`);

ICONS['Favorites folder'] = folder('fa', sheetIn(70, 42, -4)) + `
<defs>${rad('fa-st', [[0, '#fff6c0'], [.55, '#f2c01c'], [1, '#b07a04']], .4, .35, .7)}</defs>
<path d="${starPath(180, 178, 50)}" fill="#fff" stroke="#fff" stroke-width="10" stroke-linejoin="round"/>
<path d="${starPath(180, 178, 48)}" fill="url(#fa-st)" stroke="#8a5a04" stroke-width="2.5" stroke-linejoin="round"/>
<path d="${starPath(176, 168, 22)}" fill="#fff" opacity=".45"/>`;

ICONS['Recent folder'] = folder('fr', sheetIn(70, 42, -4)) + front(`
<defs>${rad('fr-fc', [[0, '#ffffff'], [.85, '#eef3f8'], [1, '#c2ccd8']], .45, .4, .6)}</defs>
<circle cx="180" cy="178" r="40" fill="url(#fr-fc)" stroke="#0b3a80" stroke-width="5"/>
<g stroke="#3a4250" stroke-linecap="round">${[0, 3, 6, 9].map(i => { const a = i * Math.PI / 6; return `<path d="M${(180 + 30 * Math.sin(a)).toFixed(1)} ${(178 - 30 * Math.cos(a)).toFixed(1)} L${(180 + 35 * Math.sin(a)).toFixed(1)} ${(178 - 35 * Math.cos(a)).toFixed(1)}" stroke-width="4"/>`; }).join('')}</g>
<path d="M180 178 V152 M180 178 L198 188" stroke="#1c2026" stroke-width="6" stroke-linecap="round"/>
${arcArrow(180, 178, 47, 200, 300, '#2a7fe0', 7, 1.3)}
<ellipse cx="172" cy="158" rx="22" ry="10" fill="#fff" opacity=".5"/>`, 180, 178, 42);
