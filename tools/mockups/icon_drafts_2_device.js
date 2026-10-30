// Icon drafts, batch 2: the device and places (icon-brief.md "The full set").
// The SD and CF cards are Juanje's (assets/cards, in the base by the owner's
// ruling of 2026-10-14), shown on the sheet as they are, not redrawn.

const cardImg = f => `<image href="../../../assets/cards/${f}" x="0" y="0" width="256" height="256"/>`;
ICONS['SD card (Juanje)'] = cardImg('card-sd.png');
ICONS['CF card (Juanje)'] = cardImg('card-compact-flash.png');

ICONS.Memory = `
${shadow(128, 206, 112, 12, 'mm-sh')}
<defs>
${lin('mm-pcb', [[0, '#c9f4ee', .9], [.5, '#3fb3a4', .8], [1, '#11574e', .92]])}
${lin('mm-au', [[0, '#fff3b0'], [.5, '#e8b020'], [1, '#8a5a04']])}
${lin('mm-ch', [[0, '#5a6474'], [.3, '#2c323c'], [1, '#0e1116']])}
</defs>
<path d="M16 70 Q16 62 24 62 L232 62 Q240 62 240 70 L240 190 L136 190 L136 182 L120 182 L120 190 L16 190 Z" fill="url(#mm-pcb)" stroke="#0e4a42" stroke-width="3"/>
${rim('M22 68 L234 68 L234 184', 2.5, .7)}
<g>${[...Array(22)].map((_, i) => { const x = 24 + i * 9.6 + (i > 10 ? 6 : 0); return `<rect x="${x.toFixed(1)}" y="166" width="6" height="22" rx="1.5" fill="url(#mm-au)" stroke="#6a4204" stroke-width="1"/>`; }).join('')}</g>
${[0, 1, 2, 3].map(i => `<rect x="${30 + i * 52}" y="80" width="40" height="66" rx="3" fill="url(#mm-ch)" stroke="#000" stroke-width="2"/><rect x="${33 + i * 52}" y="83" width="34" height="18" rx="2" fill="#fff" opacity=".25"/><circle cx="${38 + i * 52}" cy="138" r="2.5" fill="#8592a2"/>`).join('')}
${shine('M19 65 L237 65 L237 110 Q128 128 19 104 Z', 'mm-gl', .55)}
${streak(214, 72, 70, 8)}`;

ICONS.Network = `
${shadow(128, 228, 70, 10, 'nw-sh')}
<defs>${lin('nw-a', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
${[134, 92, 50].map((r, i) => { const P = a => `${(128 + r * Math.cos(a * Math.PI / 180)).toFixed(1)} ${(198 + r * Math.sin(a * Math.PI / 180)).toFixed(1)}`; return `<path d="M${P(222)} A${r} ${r} 0 0 1 ${P(318)}" fill="none" stroke="#0a2a60" stroke-width="30" stroke-linecap="round"/><path d="M${P(222)} A${r} ${r} 0 0 1 ${P(318)}" fill="none" stroke="url(#nw-a)" stroke-width="24" stroke-linecap="round"/><path d="M${P(232)} A${r + 5} ${r + 5} 0 0 1 ${P(290)}" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" opacity=".7"/>`; }).join('')}
${orb('nw-dot', 128, 198, 20, C.blue, .7)}`;

ICONS.Bluetooth = `
${shadow(128, 234, 64, 10, 'bt-sh')}
<defs>${lin('bt-bd', [[0, '#c6ecff'], [.45, '#2a7fe0'], [1, '#0b3a80']], 1, 1)}</defs>
<rect x="60" y="18" width="136" height="212" rx="68" fill="url(#bt-bd)" stroke="#0a2a60" stroke-width="3"/>
${rim('M128 22 Q192 22 192 90 V158 Q192 226 128 226', 2.5, .55)}
<path d="M94 94 L164 162 L128 200 L128 48 L164 86 L94 156" fill="none" stroke="#08245a" stroke-width="16" stroke-linejoin="round" stroke-linecap="round" opacity=".35" transform="translate(3 4)"/>
<path d="M94 94 L164 162 L128 200 L128 48 L164 86 L94 156" fill="none" stroke="#fff" stroke-width="14" stroke-linejoin="round" stroke-linecap="round"/>
${shine('M64 90 Q64 22 128 22 Q192 22 192 90 Q128 116 64 98 Z', 'bt-gl', .6)}`;

ICONS['USB / PC'] = `
${shadow(128, 228, 104, 13, 'us-sh')}
<defs>
${lin('us-bz', [[0, '#7c8696'], [.25, '#2c323c'], [1, '#0e1116']])}
${lin('us-sc', [[0, '#7cc6ff'], [1, '#0c4aa0']])}
${lin('us-sh2', [[0, '#ffffff'], [.4, '#c4ccd6'], [1, '#5c6672']], 1, 0)}
${lin('us-bd', [[0, '#8592a2'], [.4, '#3c4450'], [1, '#14181e']], 1, 0)}
</defs>
<rect x="14" y="26" width="150" height="112" rx="10" fill="url(#us-bz)" stroke="#05060a" stroke-width="3"/>
<rect x="25" y="37" width="128" height="84" rx="3" fill="url(#us-sc)"/>
<path d="M25 37 H153 V56 L25 104 Z" fill="#fff" opacity=".3"/>
<path d="M72 138 L106 138 L112 160 L66 160 Z" fill="#8592a2" stroke="#3c4450" stroke-width="2"/>
<path d="M44 162 Q50 222 128 210 Q160 204 170 186" fill="none" stroke="#2c323c" stroke-width="12" stroke-linecap="round"/>
<g transform="rotate(28 186 140)">
<rect x="164" y="128" width="44" height="66" rx="9" fill="url(#us-bd)" stroke="#05060a" stroke-width="2.5"/>
<rect x="168" y="76" width="36" height="56" rx="3" fill="url(#us-sh2)" stroke="#3c4450" stroke-width="2.5"/>
<rect x="176" y="90" width="8" height="8" fill="#3c4450"/><rect x="188" y="90" width="8" height="8" fill="#3c4450"/>
<rect x="170" y="80" width="8" height="48" rx="3" fill="#fff" opacity=".6"/>
<rect x="168" y="132" width="10" height="56" rx="4" fill="#fff" opacity=".3"/></g>`;

ICONS['Trash can, full'] = trash('bf', `
<g opacity=".9"><rect x="80" y="110" width="70" height="90" rx="3" fill="#fff" transform="rotate(-12 115 155)"/>
<circle cx="150" cy="176" r="24" fill="#e6ebf0"/></g>`, `
<rect x="104" y="6" width="60" height="78" rx="3" fill="#fbfdff" stroke="#8592a2" stroke-width="2" transform="rotate(14 134 45)"/>
${lines(116, 22, 34, 3, 14)}
<path d="M64 52 Q58 30 78 24 Q98 16 106 34 Q118 46 104 60 Z" fill="#eef2f6" stroke="#8592a2" stroke-width="2"/>
<path d="M72 36 L90 44 M84 26 L94 50" stroke="#a9b4c0" stroke-width="2"/>
<path d="M156 56 Q148 28 172 26 Q196 26 194 48 Q192 60 176 62 Z" fill="#fff4c8" stroke="#a88a3a" stroke-width="2"/>
<path d="M164 40 L184 46 M172 32 L178 56" stroke="#d7b45a" stroke-width="2"/>`);

ICONS.Desktop = `
${shadow(128, 224, 104, 12, 'dk-sh')}
<defs>
${lin('dk-bz', [[0, '#c9f4ee', .9], [.5, '#3fb3a4', .85], [1, '#11574e', .95]], 1, 1)}
${lin('dk-sk', [[0, '#5aa8ec'], [.6, '#bfe2ff'], [1, '#ffe2b0']])}
${lin('dk-hl', [[0, '#7cc85a'], [1, '#2a6a1a']])}
${lin('dk-tb', [[0, '#5c86c6'], [1, '#1a3466']])}
</defs>
<rect x="16" y="34" width="224" height="178" rx="16" fill="url(#dk-bz)" stroke="#0e4a42" stroke-width="3"/>
${rim('M32 38 H224 Q236 38 236 50', 2.5, .7)}
<rect x="30" y="48" width="196" height="150" rx="4" fill="url(#dk-sk)" stroke="#0e4a42" stroke-width="2"/>
<circle cx="190" cy="78" r="13" fill="#fff1a0"/>
<path d="M30 168 Q80 118 132 152 Q176 124 226 150 L226 182 L30 182 Z" fill="url(#dk-hl)"/>
${[64, 100, 136].map((y, i) => key('dk-i' + i, 42, y, 22, 22, [C.orange, C.blue, C.green][i], 5)).join('')}
<rect x="30" y="182" width="196" height="16" fill="url(#dk-tb)"/>
<rect x="34" y="185" width="30" height="10" rx="3" fill="#8fe07a" stroke="#1a5a10"/>
<path d="M32 50 H224 V74 L32 138 Z" fill="#fff" opacity=".28"/>`;

// Three candidate marks for the Start button (R127): RetroPane's own, nothing like a flag.
ICONS['Start: glass pane'] = `
${shadow(128, 228, 92, 12, 's1-sh')}
<defs>${lin('s1-bd', [[0, '#c9f4ee'], [.45, '#3fb3a4'], [1, '#0e4a42']], 1, 1)}</defs>
<rect x="34" y="26" width="188" height="188" rx="44" fill="url(#s1-bd)" stroke="#0a3a34" stroke-width="3" transform="rotate(-6 128 120)"/>
<g transform="rotate(-6 128 120)">${rim('M80 30 H176 Q218 30 218 72 V168 Q218 210 176 210 H80 Q38 210 38 168 V72 Q38 30 80 30 Z', 3, .75)}
<text x="128" y="170" text-anchor="middle" font-family="Arial Rounded MT Bold, Arial, sans-serif" font-weight="bold" font-size="140" fill="#08332e" opacity=".35" transform="translate(4 5)">R</text>
<text x="128" y="170" text-anchor="middle" font-family="Arial Rounded MT Bold, Arial, sans-serif" font-weight="bold" font-size="140" fill="#fff">R</text>
${shine('M40 72 Q40 32 80 32 H176 Q216 32 216 72 V88 Q128 128 40 100 Z', 's1-gl', .7)}</g>`;

ICONS['Start: orb'] = `
${shadow(128, 228, 90, 13, 's2-sh')}
<circle cx="128" cy="122" r="98" fill="none" stroke="#0a3a34" stroke-width="4"/>
${orb('s2-o', 128, 122, 94, ['#d6fff8', '#2aa898', '#063a34'], 0)}
<circle cx="128" cy="122" r="88" fill="none" stroke="#fff" stroke-width="3" opacity=".45"/>
<text x="128" y="170" text-anchor="middle" font-family="Arial Rounded MT Bold, Arial, sans-serif" font-weight="bold" font-size="132" fill="#fff">R</text>
<ellipse cx="114" cy="68" rx="64" ry="34" fill="#fff" opacity=".5" transform="rotate(-12 114 68)"/>
<ellipse cx="140" cy="196" rx="50" ry="12" fill="#bffff4" opacity=".3"/>`;

ICONS['Start: sunrise'] = `
${shadow(128, 228, 90, 13, 's3-sh')}
<defs>
<clipPath id="s3-c"><circle cx="128" cy="122" r="94"/></clipPath>
${lin('s3-sk', [[0, '#0b3a80'], [.5, '#2a7fe0'], [1, '#ffd08a']])}
${lin('s3-sea', [[0, '#3fb3a4'], [1, '#063a34']])}
</defs>
<circle cx="128" cy="122" r="98" fill="#0a2a60"/>
<g clip-path="url(#s3-c)"><rect x="20" y="20" width="220" height="220" fill="url(#s3-sk)"/>
${orb('s3-sun', 128, 140, 44, ['#fffbe0', '#ffc23a', '#e06a04'], .5)}
<rect x="20" y="140" width="220" height="100" fill="url(#s3-sea)"/>
${[152, 166, 182].map((y, i) => `<rect x="${104 - i * 10}" y="${y}" width="${48 + i * 20}" height="5" rx="2.5" fill="#ffe0a0" opacity="${.8 - i * .2}"/>`).join('')}</g>
<circle cx="128" cy="122" r="94" fill="none" stroke="#fff" stroke-width="3" opacity=".6"/>
<ellipse cx="112" cy="66" rx="62" ry="32" fill="#fff" opacity=".4" transform="rotate(-12 112 66)"/>`;
