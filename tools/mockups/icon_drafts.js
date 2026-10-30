// RetroPane icon drafts in the Crystal / Vista glass spirit: 256x256 masters.
// Each entry is SVG content for a 256 viewBox; ids are prefixed per icon.
const lin = (id, stops, x2 = 0, y2 = 1, x1 = 0, y1 = 0) =>
  `<linearGradient id="${id}" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}">${stops.map(([o, c, a]) => `<stop offset="${o}" stop-color="${c}"${a !== undefined ? ` stop-opacity="${a}"` : ''}/>`).join('')}</linearGradient>`;
const rad = (id, stops, cx = .5, cy = .5, r = .5, fx, fy) =>
  `<radialGradient id="${id}" cx="${cx}" cy="${cy}" r="${r}"${fx !== undefined ? ` fx="${fx}" fy="${fy}"` : ''}>${stops.map(([o, c, a]) => `<stop offset="${o}" stop-color="${c}"${a !== undefined ? ` stop-opacity="${a}"` : ''}/>`).join('')}</radialGradient>`;
const shadow = (cx, cy, rx, ry, id) => `<defs>${rad(id, [[0, '#000', .45], [1, '#000', 0]])}</defs><ellipse cx="${cx}" cy="${cy}" rx="${rx}" ry="${ry}" fill="url(#${id})"/>`;
const shine = (d, id, o = .75) => `<defs>${lin(id, [[0, '#fff', o], [1, '#fff', 0]])}</defs><path d="${d}" fill="url(#${id})"/>`;

const ICONS = {};

ICONS.Calendar = `
${shadow(128, 228, 104, 16, 'cal-sh')}
<defs>
${lin('cal-pg', [[0, '#ffffff'], [.7, '#eef2f7'], [1, '#c9d3df']])}
${lin('cal-hd', [[0, '#ff8a6a'], [.45, '#e53a22'], [1, '#9c160a']])}
${lin('cal-bk', [[0, '#7d8896'], [1, '#3e4652']])}
${lin('cal-ring', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#5d6874']], 1, 0)}
</defs>
<path d="M44 52 L212 52 L224 214 L32 214 Z" fill="url(#cal-bk)"/>
<path d="M40 44 L216 44 L226 206 L30 206 Z" fill="url(#cal-pg)" stroke="#8792a0" stroke-width="2"/>
<path d="M40 44 L216 44 L219 92 L37 92 Z" fill="url(#cal-hd)" stroke="#7a1008" stroke-width="2"/>
${shine('M42 46 L214 46 L215 66 Q128 76 41 66 Z', 'cal-gl', .7)}
<g fill="#c6ced8" opacity=".9">${[0, 1, 2, 3, 4, 5].map(i => [0, 1, 2].map(j => `<rect x="${58 + i * 25}" y="${152 + j * 16}" width="18" height="10" rx="2"/>`).join('')).join('')}</g>
<text x="128" y="146" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="54" fill="#2b3644">30</text>
<rect x="133" y="168" width="18" height="10" rx="2" fill="#e53a22"/>
<g>${[70, 104, 152, 186].map(x => `<rect x="${x - 6}" y="26" width="12" height="36" rx="6" fill="url(#cal-ring)" stroke="#4a5460"/>`).join('')}</g>`;

ICONS.Tasks = `
${shadow(128, 230, 92, 14, 'tk-sh')}
<defs>
${lin('tk-bd', [[0, '#5fb0ff'], [.5, '#1f6fd0'], [1, '#0d3f86']])}
${lin('tk-pp', [[0, '#ffffff'], [1, '#dfe6ee']])}
${lin('tk-cl', [[0, '#f4f6f8'], [.5, '#a6b0bc'], [1, '#5c6672']])}
${lin('tk-ck', [[0, '#9ef07a'], [.5, '#33b02a'], [1, '#13751a']])}
</defs>
<rect x="44" y="34" width="168" height="196" rx="16" fill="url(#tk-bd)" stroke="#0a2f66" stroke-width="3"/>
<rect x="60" y="56" width="136" height="160" rx="4" fill="url(#tk-pp)" stroke="#9aa6b4" stroke-width="2"/>
<rect x="92" y="20" width="72" height="40" rx="10" fill="url(#tk-cl)" stroke="#47515c" stroke-width="3"/>
<circle cx="128" cy="32" r="7" fill="#47515c"/>
${[90, 132, 174].map((y, i) => `<rect x="76" y="${y - 14}" width="26" height="26" rx="5" fill="#fff" stroke="#8592a2" stroke-width="2"/>
${i < 2 ? `<path d="M78 ${y - 2} l9 10 l20 -26" fill="none" stroke="url(#tk-ck)" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>` : ''}
<rect x="114" y="${y - 5}" width="66" height="9" rx="4" fill="${i < 2 ? '#b4bdc8' : '#7f8a98'}"/>`).join('')}
${shine('M48 40 Q128 30 208 40 L208 70 Q128 86 48 70 Z', 'tk-gl', .45)}`;

ICONS.Notes = `
${shadow(124, 230, 96, 14, 'nt-sh')}
<defs>
${lin('nt-pp', [[0, '#fff9b8'], [.6, '#ffe45a'], [1, '#e9b81c']])}
${lin('nt-sp', [[0, '#ffffff'], [.5, '#9aa4b0'], [1, '#4c5662']], 1, 0)}
${lin('nt-pc', [[0, '#ffd27a'], [.5, '#f28b1c'], [1, '#a84a08']], 1, 1)}
</defs>
<path d="M46 44 L196 44 Q206 44 206 54 L206 214 Q206 222 198 222 L54 222 Q46 222 46 214 Z" fill="#d79f12"/>
<path d="M40 36 L190 36 Q200 36 200 46 L200 206 Q200 214 192 214 L48 214 Q40 214 40 206 Z" fill="url(#nt-pp)" stroke="#a87a0a" stroke-width="2"/>
<g stroke="#d7a52a" stroke-width="3">${[86, 112, 138, 164, 190].map(y => `<path d="M58 ${y} H184"/>`).join('')}</g>
<g>${[60, 88, 116, 144, 172].map(x => `<rect x="${x}" y="24" width="10" height="28" rx="5" fill="url(#nt-sp)" stroke="#3e4650"/>`).join('')}</g>
<g transform="rotate(38 172 160)"><rect x="160" y="70" width="28" height="140" rx="5" fill="url(#nt-pc)" stroke="#7a3404" stroke-width="2"/>
<rect x="160" y="70" width="28" height="22" rx="4" fill="#f2a0b4" stroke="#a04a60" stroke-width="2"/><rect x="160" y="92" width="28" height="10" fill="#c9d0d8" stroke="#6a737e"/>
<path d="M160 210 L174 244 L188 210 Z" fill="#f4d9a8" stroke="#8a6a3a" stroke-width="2"/><path d="M169 232 L174 244 L179 232 Z" fill="#333"/>
<rect x="166" y="104" width="6" height="104" fill="#fff" opacity=".45"/></g>
${shine('M42 38 L198 38 L198 64 Q120 76 42 64 Z', 'nt-gl', .5)}`;

ICONS.Inbox = `
${shadow(128, 222, 104, 14, 'ib-sh')}
<defs>
${lin('ib-bd', [[0, '#ffffff'], [.6, '#e4eaf1'], [1, '#b7c3d1']])}
${lin('ib-fl', [[0, '#9fd2ff'], [.5, '#3d8fe6'], [1, '#1a59b0']])}
${rad('ib-bg', [[0, '#ff9a8a'], [.6, '#e3321e'], [1, '#9a1206']], .4, .35, .7)}
</defs>
<path d="M28 76 L228 76 L228 200 Q228 208 220 208 L36 208 Q28 208 28 200 Z" fill="url(#ib-bd)" stroke="#6b7888" stroke-width="3"/>
<path d="M28 200 L108 132 M228 200 L148 132" stroke="#9aa8b8" stroke-width="3"/>
<path d="M28 76 L128 154 L228 76 Z" fill="url(#ib-fl)" stroke="#123f80" stroke-width="3" stroke-linejoin="round"/>
${shine('M34 80 L222 80 L128 150 Z', 'ib-gl', .55)}
<circle cx="204" cy="70" r="30" fill="url(#ib-bg)" stroke="#fff" stroke-width="6"/>
<text x="204" y="83" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="36" fill="#fff">3</text>`;

const folder = (p, inner = '') => `
${shadow(128, 226, 108, 14, p + '-sh')}
<defs>
${lin(p + '-bk', [[0, '#9fe6dc'], [.5, '#3fb3a4'], [1, '#18786c']])}
${lin(p + '-fr', [[0, '#c9f4ee', .85], [.5, '#62c9bb', .78], [1, '#1d8a7c', .92]])}
${lin(p + '-ed', [[0, '#ffffff', .9], [1, '#ffffff', 0]])}
</defs>
<path d="M30 52 Q30 42 40 42 L100 42 Q108 42 112 50 L120 62 L214 62 Q224 62 224 72 L224 206 L30 206 Z" fill="url(#${p}-bk)" stroke="#11574e" stroke-width="3"/>
<path d="M38 70 L218 70 L218 76 L38 76 Z" fill="#ffffff" opacity=".7"/>
${inner}
<path d="M22 100 Q22 92 30 92 L226 92 Q234 92 233 100 L222 208 Q221 216 213 216 L40 216 Q32 216 31 208 Z" fill="url(#${p}-fr)" stroke="#11574e" stroke-width="3"/>
<path d="M30 96 L226 96 L224 116 Q128 128 30 116 Z" fill="url(#${p}-ed)" opacity=".7"/>
<path d="M36 200 L218 200" stroke="#ffffff" stroke-width="2" opacity=".5"/>`;

ICONS.Folder = folder('fo', `<rect x="56" y="54" width="132" height="70" rx="3" fill="#fff" stroke="#9aa6b4" stroke-width="2" transform="rotate(-4 120 90)"/>`);

ICONS.Music = folder('mu', `
<defs>${lin('mu-nt', [[0, '#8fd0ff'], [.5, '#2a7fe0'], [1, '#0c3c88']])}</defs>
<path d="M124 40 L190 24 L190 132 A20 16 0 1 1 174 116 L174 54 L140 62 L140 150 A20 16 0 1 1 124 134 Z" fill="url(#mu-nt)" stroke="#0a2a60" stroke-width="3"/>
<path d="M130 46 L184 34 L184 44 L130 56 Z" fill="#fff" opacity=".6"/>`);

ICONS.Browser = `
${shadow(128, 226, 92, 14, 'br-sh')}
<defs>
${rad('br-gl', [[0, '#d8f1ff'], [.35, '#5fb4ff'], [.8, '#1257b8'], [1, '#062c6a']], .38, .32, .75)}
${lin('br-ld', [[0, '#b6f08a'], [.5, '#3fae2c'], [1, '#1a6a14']])}
${lin('br-rg', [[0, '#ffe08a'], [.5, '#f2a01c'], [1, '#b05e04']], 1, 0)}
</defs>
<circle cx="128" cy="120" r="92" fill="url(#br-gl)" stroke="#05285e" stroke-width="3"/>
<path d="M70 66 Q96 48 120 62 Q132 80 112 92 Q98 104 108 122 Q118 140 96 154 Q78 160 70 140 Q60 120 52 100 Q54 80 70 66 Z M150 50 Q176 54 196 80 Q206 104 190 112 Q172 110 160 96 Q150 80 140 70 Q140 56 150 50 Z M148 150 Q170 138 190 150 Q186 176 160 196 Q146 194 148 176 Z" fill="url(#br-ld)" stroke="#145a10" stroke-width="2" opacity=".95"/>
<ellipse cx="104" cy="70" rx="58" ry="32" fill="#fff" opacity=".42" transform="rotate(-20 104 70)"/>
<ellipse cx="128" cy="122" rx="122" ry="34" fill="none" stroke="url(#br-rg)" stroke-width="12" transform="rotate(-18 128 122)" stroke-dasharray="300 70" stroke-dashoffset="40"/>`;

ICONS.Media = `
${shadow(128, 228, 96, 14, 'md-sh')}
<defs>
${rad('md-dk', [[0, '#ffffff'], [.25, '#e3e8ee'], [.55, '#b8c4d2'], [.75, '#e8eef4'], [1, '#8592a2']], .45, .42, .62)}
${rad('md-pl', [[0, '#ffd08a'], [.55, '#f27a12'], [1, '#a03c02']], .4, .35, .75)}
</defs>
<circle cx="112" cy="124" r="88" fill="url(#md-dk)" stroke="#55616e" stroke-width="3"/>
<path d="M60 70 A80 80 0 0 1 150 50" stroke="#ff9ad8" stroke-width="10" fill="none" opacity=".35"/>
<path d="M54 110 A64 64 0 0 1 96 64" stroke="#8ae0ff" stroke-width="10" fill="none" opacity=".35"/>
<circle cx="112" cy="124" r="22" fill="#f4f7fa" stroke="#6e7a88" stroke-width="3"/><circle cx="112" cy="124" r="8" fill="#5c6672"/>
<circle cx="182" cy="168" r="50" fill="url(#md-pl)" stroke="#7a2c02" stroke-width="3"/>
<path d="M168 144 L206 168 L168 192 Z" fill="#fff"/>
<ellipse cx="174" cy="146" rx="32" ry="16" fill="#fff" opacity=".45"/>`;

ICONS.Games = `
${shadow(128, 216, 112, 14, 'gm-sh')}
<defs>
${lin('gm-bd', [[0, '#6c7584'], [.4, '#2c323c'], [1, '#0e1116']])}
${rad('gm-a', [[0, '#ffb2a6'], [.6, '#e3321e'], [1, '#8a1004']], .4, .35, .7)}
${rad('gm-b', [[0, '#b6e4ff'], [.6, '#2a7fe0'], [1, '#0b3a80']], .4, .35, .7)}
${rad('gm-c', [[0, '#c8f7a8'], [.6, '#36b02a'], [1, '#126810']], .4, .35, .7)}
${rad('gm-d', [[0, '#fff2a8'], [.6, '#f2c01c'], [1, '#9a6a04']], .4, .35, .7)}
</defs>
<path d="M58 78 Q128 60 198 78 Q232 86 238 140 Q244 196 210 200 Q190 202 170 168 L86 168 Q66 202 46 200 Q12 196 18 140 Q24 86 58 78 Z" fill="url(#gm-bd)" stroke="#050608" stroke-width="3"/>
<path d="M62 84 Q128 68 194 84 Q214 90 222 110 Q128 96 34 110 Q42 90 62 84 Z" fill="#fff" opacity=".28"/>
<path d="M62 116 h16 v-16 h16 v16 h16 v16 h-16 v16 h-16 v-16 h-16 z" fill="#c9d0d8" stroke="#3a4048" stroke-width="2"/>
<circle cx="184" cy="104" r="12" fill="url(#gm-c)"/><circle cx="208" cy="126" r="12" fill="url(#gm-a)"/>
<circle cx="160" cy="126" r="12" fill="url(#gm-b)"/><circle cx="184" cy="148" r="12" fill="url(#gm-d)"/>`;

ICONS.Settings = `
${shadow(128, 226, 92, 14, 'st-sh')}
<defs>
${lin('st-gr', [[0, '#ffffff'], [.4, '#c3ccd6'], [.7, '#8592a2'], [1, '#4c5664']])}
${rad('st-cn', [[0, '#c4e8ff'], [.5, '#2f86e0'], [1, '#0b3a80']], .4, .35, .7)}
</defs>
<g transform="translate(128 122)">
<path d="${(() => { let d = ''; const n = 10; for (let i = 0; i < n; i++) { const a = i / n * Math.PI * 2, b = a + Math.PI / n * .55, c = a + Math.PI / n * 1.45, e = a + Math.PI / n * 2; const P = (r, t) => `${(r * Math.cos(t)).toFixed(1)} ${(r * Math.sin(t)).toFixed(1)}`; d += (i ? 'L' : 'M') + P(78, a) + 'L' + P(96, b) + 'L' + P(96, c) + 'L' + P(78, e); } return d + 'Z'; })()}" fill="url(#st-gr)" stroke="#3a4250" stroke-width="3"/>
<circle r="46" fill="url(#st-cn)" stroke="#0a2a60" stroke-width="3"/>
<circle r="18" fill="#e8eef4" stroke="#3a4250" stroke-width="3"/>
<ellipse cx="-14" cy="-24" rx="26" ry="12" fill="#fff" opacity=".5"/></g>`;

ICONS['My Device'] = `
${shadow(128, 234, 70, 12, 'dv-sh')}
<defs>
${lin('dv-bd', [[0, '#8a95a4'], [.15, '#3c4450'], [.85, '#1a1e24'], [1, '#4a5260']], 1, 0)}
${lin('dv-sc', [[0, '#bfe9ff'], [.5, '#3c9cf0'], [1, '#0c4aa0']])}
${rad('dv-bt', [[0, '#e8eef4'], [1, '#6c7886']], .4, .35, .7)}
</defs>
<rect x="66" y="18" width="124" height="208" rx="18" fill="url(#dv-bd)" stroke="#0a0c10" stroke-width="3"/>
<rect x="80" y="38" width="96" height="124" rx="3" fill="url(#dv-sc)" stroke="#05080c" stroke-width="2"/>
<path d="M80 128 Q128 100 176 118 L176 162 L80 162 Z" fill="#5fc46a" opacity=".55"/>
<circle cx="150" cy="64" r="12" fill="#ffe27a"/>
<path d="M80 38 L176 38 L176 80 Q128 96 80 80 Z" fill="#fff" opacity=".35"/>
<circle cx="128" cy="194" r="17" fill="url(#dv-bt)" stroke="#2a3038" stroke-width="2"/>
${[92, 164].map(x => `<rect x="${x - 10}" y="186" width="20" height="14" rx="4" fill="#5c6672" stroke="#2a3038"/>`).join('')}
<rect x="70" y="22" width="10" height="200" rx="5" fill="#fff" opacity=".18"/>`;

ICONS['Deleted Items'] = `
${shadow(128, 228, 84, 14, 'bn-sh')}
<defs>
${lin('bn-gl', [[0, '#d9f6f2', .85], [.5, '#76d0c4', .7], [1, '#2a9486', .9]], 1, 0)}
${lin('bn-rm', [[0, '#ffffff'], [1, '#7fc9be']])}
</defs>
<path d="M54 64 L202 64 L184 216 Q182 224 174 224 L82 224 Q74 224 72 216 Z" fill="url(#bn-gl)" stroke="#155e55" stroke-width="3"/>
<ellipse cx="128" cy="64" rx="74" ry="14" fill="url(#bn-rm)" stroke="#155e55" stroke-width="3"/>
<g fill="none" stroke="#ffffff" stroke-width="7" stroke-linecap="round" opacity=".85">
<path d="M112 120 l16 -22 l16 22"/><path d="M150 140 l12 26 l-26 2"/><path d="M106 168 l-26 -2 l12 -26"/></g>
<path d="M66 82 L88 82 L96 208 L84 208 Z" fill="#fff" opacity=".35"/>`;
