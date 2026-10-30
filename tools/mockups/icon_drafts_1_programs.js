// Icon drafts, batch 1: the programs (icon-brief.md "The full set").
// Uses the helpers in icon_drafts.js (loaded first).

ICONS.Today = `
${shadow(128, 228, 104, 14, 'td-sh')}
<defs>
${lin('td-wl', [[0, '#e4fbf8', .9], [.5, '#76d0c4', .75], [1, '#1d8a7c', .92]])}
${lin('td-rf', [[0, '#ffb08a'], [.5, '#e8502a'], [1, '#a0240a']])}
${lin('td-dr', [[0, '#ffe7a0'], [1, '#c07a10']])}
${lin('td-wn', [[0, '#e6f6ff'], [1, '#2a7fe0']])}
</defs>
${orb('td-sun', 186, 62, 40, ['#fffbe0', '#ffd23a', '#d27a04'])}
<path d="M58 118 L198 118 L198 216 L58 216 Z" fill="url(#td-wl)" stroke="#11574e" stroke-width="3"/>
${rim('M63 124 L193 124 L193 211 L63 211 Z', 2.5, .7)}
${[70, 160].map(x => `<rect x="${x}" y="146" width="28" height="28" rx="3" fill="url(#td-wn)" stroke="#0b3a80" stroke-width="2"/><path d="M${x + 14} 146 V174 M${x} 160 H${x + 28}" stroke="#fff" stroke-width="2.5"/>`).join('')}
<rect x="110" y="156" width="36" height="60" rx="4" fill="url(#td-dr)" stroke="#7a4a04" stroke-width="2"/>
<circle cx="138" cy="188" r="3.5" fill="#7a4a04"/>
<path d="M28 126 L128 44 L228 126 Q232 134 222 134 L34 134 Q24 134 28 126 Z" fill="url(#td-rf)" stroke="#7a1008" stroke-width="3" stroke-linejoin="round"/>
${shine('M40 124 L128 52 L170 86 Q110 98 44 128 Z', 'td-gl', .75)}
${glow('M60 192 L196 192 L196 214 L60 214 Z', 'td-gw', .5)}`;

ICONS.Contacts = `
${shadow(124, 232, 96, 14, 'ct-sh')}
<defs>
${lin('ct-bk', [[0, '#8fd0ff', .9], [.5, '#2f7fe0', .85], [1, '#0d3f86', .95]], 1, 1)}
${lin('ct-pg', [[0, '#ffffff'], [1, '#dfe7f0']])}
${rad('ct-ps', [[0, '#d8f0ff'], [.5, '#3a92ea'], [1, '#0b3a80']], .38, .3, .8)}
</defs>
<rect x="34" y="26" width="172" height="202" rx="14" fill="url(#ct-bk)" stroke="#0a2f66" stroke-width="3"/>
${rim('M48 29 H192 Q203 29 203 40 V214 Q203 225 192 225 H48', 2.5, .7)}
${[['#f58020', 52], ['#36b02a', 90], ['#3fb3a4', 128], ['#e3321e', 166]].map(([c, y]) => `<rect x="196" y="${y}" width="28" height="30" rx="6" fill="${c}" stroke="#333" stroke-width="1.5"/><rect x="200" y="${y + 3}" width="20" height="10" rx="4" fill="#fff" opacity=".5"/>`).join('')}
<rect x="58" y="42" width="136" height="170" rx="6" fill="url(#ct-pg)" stroke="#8592a2" stroke-width="2"/>
<path d="M78 196 Q78 142 126 142 Q174 142 174 196 Z" fill="url(#ct-ps)" stroke="#0a2a60" stroke-width="2.5"/>
<circle cx="126" cy="100" r="32" fill="url(#ct-ps)" stroke="#0a2a60" stroke-width="2.5"/>
<ellipse cx="118" cy="84" rx="18" ry="10" fill="#fff" opacity=".6"/>
<ellipse cx="114" cy="154" rx="22" ry="7" fill="#fff" opacity=".45"/>
<g>${[44, 76, 108, 140, 172, 204].map(y => `<rect x="26" y="${y - 6}" width="22" height="12" rx="6" fill="#d6dce4" stroke="#4a5460"/>`).join('')}</g>
${shine('M37 29 L204 29 L204 80 Q120 104 37 90 Z', 'ct-gl', .45)}`;

ICONS.Documents = `<g transform="translate(12 0)">` + page('dc', `
<rect x="70" y="58" width="80" height="12" rx="5" fill="#2a7fe0"/>
${lines(70, 84, 116, 6, 18)}
<defs>${lin('dc-bd', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
<rect x="120" y="138" width="96" height="96" rx="18" fill="url(#dc-bd)" stroke="#0a2a60" stroke-width="3"/>
<rect x="125" y="142" width="86" height="38" rx="14" fill="#fff" opacity=".45"/>
<text x="168" y="212" text-anchor="middle" font-family="Georgia, serif" font-weight="bold" font-size="72" fill="#fff">A</text>`, 34, 22) + '</g>';

ICONS.Spreadsheet = `<g transform="translate(14 0)">` + page('ss', `
<g stroke="#b4bdc8" stroke-width="2">${[0, 1, 2, 3, 4, 5, 6].map(i => `<path d="M70 ${70 + i * 22} H170"/>`).join('')}${[0, 1, 2, 3].map(i => `<path d="M${70 + i * 33.3} 70 V202"/>`).join('')}</g>
<rect x="70" y="70" width="100" height="22" fill="#36b02a" opacity=".85"/>
${key('ss-b1', 120, 168, 26, 58, C.green, 4)}${key('ss-b2', 152, 136, 26, 90, C.green, 4)}${key('ss-b3', 184, 100, 26, 126, C.green, 4)}`, 34, 22) + '</g>';

ICONS.Presentation = `
${shadow(128, 234, 88, 12, 'pr-sh')}
<defs>
${lin('pr-lg', [[0, '#d4dbe3'], [.5, '#8592a2'], [1, '#3c4450']], 1, 0)}
${lin('pr-bd', [[0, '#ffffff'], [.8, '#eef3f8'], [1, '#c8d3df']])}
${lin('pr-sl', [[0, '#ffe0b0'], [.5, '#f58020'], [1, '#a03c02']])}
${rad('pr-pi', [[0, '#c6ecff'], [.6, '#2a7fe0'], [1, '#0b3a80']], .4, .35, .75)}
</defs>
<g stroke="url(#pr-lg)" stroke-width="10" stroke-linecap="round"><path d="M98 180 L70 232"/><path d="M158 180 L186 232"/><path d="M128 180 L128 226"/></g>
<rect x="26" y="38" width="204" height="146" rx="6" fill="url(#pr-bd)" stroke="#6d7886" stroke-width="3"/>
<rect x="18" y="28" width="220" height="16" rx="8" fill="url(#pr-lg)" stroke="#3c4450" stroke-width="2"/>
<circle cx="96" cy="114" r="48" fill="url(#pr-pi)" stroke="#0a2a60" stroke-width="2.5"/>
<path d="M106 104 L106 56 A48 48 0 0 1 152.4 116.4 Z" fill="url(#pr-sl)" stroke="#7a2c02" stroke-width="2.5" stroke-linejoin="round"/>
<ellipse cx="84" cy="88" rx="22" ry="11" fill="#fff" opacity=".5"/>
${lines(166, 72, 46, 5, 20)}
${shine('M29 47 L227 47 L227 92 Q128 120 29 104 Z', 'pr-gl', .55)}`;

ICONS['Pictures & Videos'] = `
${shadow(128, 228, 104, 14, 'pv-sh')}
<defs>
${lin('pv-sk', [[0, '#7cc6ff'], [1, '#d8f0ff']])}
${lin('pv-hl', [[0, '#8fe07a'], [1, '#2a8a20']])}
${lin('pv-fm', [[0, '#4a525e'], [1, '#14181e']])}
</defs>
<g transform="rotate(-14 128 100)">
<rect x="30" y="40" width="190" height="96" rx="5" fill="url(#pv-fm)" stroke="#05060a" stroke-width="2"/>
${[...Array(10)].map((_, i) => `<rect x="${38 + i * 18}" y="46" width="9" height="9" rx="2" fill="#dfe5ec"/><rect x="${38 + i * 18}" y="122" width="9" height="9" rx="2" fill="#dfe5ec"/>`).join('')}
${[0, 1, 2].map(i => `<rect x="${40 + i * 60}" y="62" width="52" height="54" rx="2" fill="#3a92ea" opacity=".7"/>`).join('')}
<rect x="32" y="42" width="186" height="20" rx="4" fill="#fff" opacity=".18"/></g>
<g transform="rotate(5 128 156)">
<rect x="44" y="92" width="170" height="130" rx="5" fill="#fbfdff" stroke="#7d8896" stroke-width="2.5"/>
<rect x="56" y="104" width="146" height="94" fill="url(#pv-sk)" stroke="#8592a2"/>
<circle cx="176" cy="126" r="12" fill="#ffe27a"/>
<path d="M56 180 Q90 140 122 168 Q152 138 202 172 L202 198 L56 198 Z" fill="url(#pv-hl)"/>
${shine('M46 94 L212 94 L212 132 Q128 152 46 140 Z', 'pv-gl', .55)}</g>`;

ICONS['File Explorer'] = folder('fx', `<rect x="56" y="54" width="132" height="70" rx="3" fill="#fff" stroke="#9aa6b4" stroke-width="2" transform="rotate(-4 120 90)"/>`) + lens('fx-l', 150, 152, 38);

ICONS.Calculator = `
${shadow(128, 234, 84, 12, 'ca-sh')}
<defs>
${lin('ca-bd', [[0, '#8fb6e8', .95], [.4, '#3a5a8a', .95], [1, '#14223a']], 1, 1)}
${lin('ca-lc', [[0, '#e8fff4'], [1, '#9fd4bc']])}
</defs>
<rect x="50" y="18" width="156" height="212" rx="20" fill="url(#ca-bd)" stroke="#0a1426" stroke-width="3"/>
${rim('M68 21 H188 Q203 21 203 36', 2.5, .65)}
<rect x="66" y="36" width="124" height="48" rx="6" fill="url(#ca-lc)" stroke="#0a1426" stroke-width="2.5"/>
<text x="180" y="72" text-anchor="end" font-family="Arial, sans-serif" font-weight="bold" font-size="32" fill="#1d3a2e">3.14</text>
<path d="M68 38 H188 V54 Q128 62 68 54 Z" fill="#fff" opacity=".45"/>
${[0, 1, 2, 3].map(r => [0, 1, 2, 3].map(c => key(`ca-k${r}${c}`, 66 + c * 32, 98 + r * 32, 26, 24, c == 3 ? C.orange : C.silver, 6)).join('')).join('')}
${shine('M53 21 L203 21 L203 70 Q128 92 53 80 Z', 'ca-gl', .35)}`;

const suit = {
  heart: s => `M0 ${s} C ${-1.4 * s} 0, ${-.8 * s} ${-1.1 * s}, 0 ${-.4 * s} C ${.8 * s} ${-1.1 * s}, ${1.4 * s} 0, 0 ${s} Z`,
  spade: s => `M0 ${-s} C ${-1.4 * s} 0, ${-.8 * s} ${1.1 * s}, 0 ${.4 * s} C ${.8 * s} ${1.1 * s}, ${1.4 * s} 0, 0 ${-s} Z M0 ${.2 * s} L ${-.4 * s} ${1.1 * s} L ${.4 * s} ${1.1 * s} Z`,
};
const card = (p, tx, ty, r, kind, col) => `<g transform="translate(${tx} ${ty}) rotate(${r})">
<rect x="-56" y="-78" width="112" height="156" rx="10" fill="url(#${p}-cd)" stroke="#6d7886" stroke-width="2.5"/>
<text x="-42" y="-50" font-family="Arial, sans-serif" font-weight="bold" font-size="26" fill="${col}">A</text>
<path d="${suit[kind](34)}" fill="url(#${p}-${kind})" stroke="${kind == 'heart' ? '#7a0a04' : '#000'}" stroke-width="2" transform="translate(4 8)"/>
<ellipse cx="-6" cy="-10" rx="16" ry="7" fill="#fff" opacity=".5"/>
<path d="M-53 -75 H53 V-30 Q0 -14 -53 -24 Z" fill="#fff" opacity=".35"/></g>`;
ICONS.Solitaire = `
${shadow(128, 230, 100, 14, 'so-sh')}
<defs>
${lin('so-cd', [[0, '#ffffff'], [1, '#e2e9f1']])}
${lin('so-heart', [[0, '#ff9c8a'], [.5, '#e3321e'], [1, '#8a1004']])}
${lin('so-spade', [[0, '#7c8696'], [.5, '#2c323c'], [1, '#0e1116']])}
</defs>
${card('so', 80, 112, -16, 'spade', '#1c2026')}
${card('so', 168, 136, 12, 'heart', '#d0281a')}`;

ICONS['Bubble Breaker'] = `
${shadow(128, 226, 104, 14, 'bb-sh')}
${[['red', 80, 184], ['blue', 128, 184], ['green', 176, 184], ['yellow', 104, 140], ['purple', 152, 140], ['orange', 128, 96]].map(([c, x, y], i) => orb('bb-' + i, x, y, 26, C[c])).join('')}
${orb('bb-6', 188, 62, 16, C.blue, .7)}${orb('bb-7', 66, 76, 11, C.red, .7)}`;

ICONS['Clock & Alarms'] = `
${shadow(128, 234, 90, 12, 'ck-sh')}
<defs>
${rad('ck-bd', [[0, '#ffb0a2'], [.55, '#e3321e'], [1, '#8a1004']], .38, .3, .8)}
${rad('ck-fc', [[0, '#ffffff'], [.85, '#eef3f8'], [1, '#c2ccd8']], .45, .4, .6)}
${lin('ck-lg', [[0, '#d4dbe3'], [1, '#4c5664']], 1, 0)}
</defs>
<g stroke="url(#ck-lg)" stroke-width="12" stroke-linecap="round"><path d="M82 196 L62 230"/><path d="M174 196 L194 230"/></g>
<path d="M92 36 Q128 18 164 36" fill="none" stroke="#4c5664" stroke-width="8" stroke-linecap="round"/>
${orb('ck-b1', 66, 56, 30, C.silver)}${orb('ck-b2', 190, 56, 30, C.silver)}
<circle cx="128" cy="134" r="86" fill="url(#ck-bd)" stroke="#6a0a02" stroke-width="3"/>
${rim('M128 52 A82 82 0 0 1 210 134', 2.5, .6)}
<circle cx="128" cy="134" r="68" fill="url(#ck-fc)" stroke="#6a0a02" stroke-width="2.5"/>
<g stroke="#3a4250" stroke-linecap="round">${[...Array(12)].map((_, i) => { const a = i * Math.PI / 6, m = i % 3 == 0, r0 = m ? 50 : 55; return `<path d="M${(128 + r0 * Math.sin(a)).toFixed(1)} ${(134 - r0 * Math.cos(a)).toFixed(1)} L${(128 + 61 * Math.sin(a)).toFixed(1)} ${(134 - 61 * Math.cos(a)).toFixed(1)}" stroke-width="${m ? 6 : 3}"/>`; }).join('')}</g>
<path d="M128 134 L98.6 117" stroke="#1c2026" stroke-width="9" stroke-linecap="round"/>
<path d="M128 134 L171.3 109" stroke="#1c2026" stroke-width="6" stroke-linecap="round"/>
<path d="M128 134 L128 182" stroke="#e3321e" stroke-width="3" stroke-linecap="round"/>
<circle cx="128" cy="134" r="7" fill="#e3321e" stroke="#6a0a02" stroke-width="2"/>
<ellipse cx="110" cy="96" rx="44" ry="20" fill="#fff" opacity=".5" transform="rotate(-18 110 96)"/>`;

ICONS.Weather = `
${shadow(128, 226, 100, 13, 'we-sh')}
<defs>
${lin('we-cl', [[0, '#ffffff'], [.6, '#eef4fa'], [1, '#a9bed4']])}
</defs>
${orb('we-sun', 152, 92, 56, ['#fffbe0', '#ffd23a', '#d27a04'], .55)}
<path d="M58 206 Q24 206 24 174 Q24 144 56 142 Q60 106 96 102 Q124 100 138 124 Q150 110 172 116 Q198 124 198 152 Q226 156 226 180 Q226 206 198 206 Z" fill="url(#we-cl)" stroke="#6f8aa6" stroke-width="3" stroke-linejoin="round"/>
${shine('M30 168 Q32 148 58 148 Q64 112 96 108 Q122 108 134 132 Q150 116 170 122 Q192 130 192 156 Q120 176 30 168 Z', 'we-gl', .85)}
${glow('M30 190 L222 190 Q222 204 198 204 L58 204 Q30 204 30 190 Z', 'we-gw', .6)}`;

ICONS.Reader = `
${shadow(128, 228, 110, 14, 'rd-sh')}
<defs>
${lin('rd-cv', [[0, '#8fd0ff'], [.5, '#2f7fe0'], [1, '#0d3f86']])}
${lin('rd-pl', [[0, '#ffffff'], [.8, '#eef3f8'], [1, '#c8d3df']], 1, 0)}
${lin('rd-pr', [[0, '#c8d3df'], [.2, '#eef3f8'], [1, '#ffffff']], 1, 0)}
${lin('rd-rb', [[0, '#ff9c8a'], [.5, '#e3321e'], [1, '#8a1004']], 1, 0)}
</defs>
<path d="M16 84 L128 106 L240 84 L240 206 L128 228 L16 206 Z" fill="url(#rd-cv)" stroke="#0a2f66" stroke-width="3" stroke-linejoin="round"/>
<path d="M26 74 Q80 58 128 88 L128 214 Q80 188 26 198 Z" fill="url(#rd-pl)" stroke="#7d8896" stroke-width="2.5" stroke-linejoin="round"/>
<path d="M128 88 Q176 58 230 74 L230 198 Q176 188 128 214 Z" fill="url(#rd-pr)" stroke="#7d8896" stroke-width="2.5" stroke-linejoin="round"/>
<g fill="none" stroke="#b4bdc8" stroke-width="5" stroke-linecap="round">${[0, 1, 2, 3, 4, 5].map(k => { const d = 20 + k * 18; return `<path d="M42 ${76 + d} Q80 ${64 + d} 116 ${88 + d}"/><path d="M140 ${88 + d} Q176 ${64 + d} ${k == 5 ? 190 : 214} ${76 + d}"/>`; }).join('')}</g>
<path d="M176 70 L176 236 L186 226 L196 236 L196 69" fill="url(#rd-rb)" stroke="#6a0a02" stroke-width="2" stroke-linejoin="round"/>
${shine('M28 76 Q80 60 128 90 Q176 60 228 76 L228 110 Q176 96 128 126 Q80 96 28 110 Z', 'rd-gl', .5)}`;

ICONS.Record = `
${shadow(128, 232, 70, 11, 'rc-sh')}
<defs>
${lin('rc-cp', [[0, '#ffffff'], [.35, '#c4ccd6'], [.7, '#6c7886'], [1, '#2c323c']], 1, 0)}
${lin('rc-ms', [[0, '#5a6474'], [1, '#1a1e24']], 1, 0)}
${lin('rc-st', [[0, '#e6ebf0'], [.5, '#8592a2'], [1, '#3c4450']], 1, 0)}
</defs>
<path d="M68 108 Q68 176 128 176 Q188 176 188 108" fill="none" stroke="url(#rc-st)" stroke-width="11" stroke-linecap="round"/>
<rect x="121" y="174" width="14" height="36" fill="url(#rc-st)" stroke="#3c4450" stroke-width="1.5"/>
<ellipse cx="128" cy="214" rx="50" ry="13" fill="url(#rc-st)" stroke="#3c4450" stroke-width="2"/>
<rect x="84" y="20" width="88" height="136" rx="44" fill="url(#rc-cp)" stroke="#1a1e24" stroke-width="3"/>
<path d="M88 64 A40 40 0 0 1 168 64 L168 100 L88 100 Z" fill="url(#rc-ms)"/>
<g stroke="#8592a2" stroke-width="2" opacity=".8">${[40, 52, 64, 76, 88].map(y => `<path d="M${y < 52 ? 104 : 90} ${y} H${y < 52 ? 152 : 166}"/>`).join('')}${[100, 114, 128, 142, 156].map(x => `<path d="M${x} 30 V100"/>`).join('')}</g>
<rect x="84" y="100" width="88" height="10" fill="#d4dbe3" stroke="#1a1e24" stroke-width="1.5"/>
<rect x="94" y="28" width="16" height="118" rx="8" fill="#fff" opacity=".4"/>
<circle cx="190" cy="180" r="36" fill="#fff"/>
${orb('rc-dot', 190, 180, 30, C.red)}`;

ICONS.Search = lens('se', 108, 104, 66);

ICONS.Help = `
${shadow(128, 226, 90, 14, 'hp-sh')}
${orb('hp-o', 128, 122, 92, C.blue, 0)}
<text x="128" y="170" text-anchor="middle" font-family="Georgia, serif" font-weight="bold" font-size="140" fill="#08245a" opacity=".35" transform="translate(4 5)">?</text>
<text x="128" y="170" text-anchor="middle" font-family="Georgia, serif" font-weight="bold" font-size="140" fill="#fff">?</text>
<ellipse cx="112" cy="70" rx="58" ry="30" fill="#fff" opacity=".5" transform="rotate(-14 112 70)"/>
<ellipse cx="140" cy="190" rx="44" ry="10" fill="#fff" opacity=".22"/>`;

ICONS.LegacySync = `
${shadow(128, 230, 94, 13, 'ls-sh')}
<defs>
${lin('ls-g', [[0, '#d6fab8'], [.5, '#36b02a'], [1, '#126810']])}
${lin('ls-b', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}
${lin('ls-dv', [[0, '#9aa6b6'], [.2, '#3c4450'], [1, '#14181e']], 1, 0)}
${lin('ls-sc', [[0, '#cdeeff'], [1, '#2a7fe0']])}
</defs>
${arcArrow(128, 124, 86, 196, 330, 'url(#ls-g)', 22)}
${arcArrow(128, 124, 86, 16, 150, 'url(#ls-b)', 22)}
<rect x="104" y="80" width="48" height="86" rx="8" fill="url(#ls-dv)" stroke="#05060a" stroke-width="2.5"/>
<rect x="110" y="88" width="36" height="46" rx="2" fill="url(#ls-sc)"/>
<path d="M110 88 H146 V104 L110 124 Z" fill="#fff" opacity=".45"/>
<circle cx="128" cy="150" r="7" fill="#d4dbe3"/>
<path d="M60 74 A86 86 0 0 1 180 54" fill="none" stroke="#fff" stroke-width="5" opacity=".55" stroke-linecap="round"/>
<path d="M190 176 A86 86 0 0 1 100 205" fill="none" stroke="#fff" stroke-width="5" opacity=".45" stroke-linecap="round"/>`;

ICONS['Remote Desktop'] = `
${shadow(128, 226, 96, 12, 'rm-sh')}
<defs>
${lin('rm-bz', [[0, '#7c8696'], [.25, '#2c323c'], [1, '#0e1116']])}
${lin('rm-sc', [[0, '#7cc6ff'], [1, '#0c4aa0']])}
${lin('rm-st', [[0, '#e6ebf0'], [.5, '#8592a2'], [1, '#3c4450']], 1, 0)}
</defs>
<path d="M104 178 L152 178 L162 212 L94 212 Z" fill="url(#rm-st)" stroke="#3c4450" stroke-width="2"/>
<ellipse cx="128" cy="214" rx="56" ry="10" fill="url(#rm-st)" stroke="#3c4450" stroke-width="2"/>
<rect x="20" y="30" width="216" height="154" rx="12" fill="url(#rm-bz)" stroke="#05060a" stroke-width="3"/>
${rim('M34 33 H222 Q233 33 233 44', 2.5, .55)}
<rect x="34" y="44" width="188" height="124" rx="3" fill="url(#rm-sc)" stroke="#05060a" stroke-width="2"/>
<rect x="56" y="64" width="104" height="72" rx="3" fill="#fbfdff" stroke="#0b3a80" stroke-width="2"/>
<rect x="56" y="64" width="104" height="16" rx="3" fill="#2a7fe0"/>
${lines(66, 90, 80, 3, 14)}
<path d="M34 44 L222 44 L222 70 L34 132 Z" fill="#fff" opacity=".3"/>
<circle cx="194" cy="176" r="40" fill="#fff"/>
${orb('rm-b', 194, 176, 34, C.orange, .5)}
<path d="M176 190 Q176 166 200 166 L200 154 L220 174 L200 194 L200 182 Q188 182 176 190 Z" fill="#fff" stroke="#fff" stroke-width="2" stroke-linejoin="round"/>`;

// The first take, a tile grid (not liked by the owner).
ICONS['Programs: tiles'] = `
${shadow(128, 230, 104, 14, 'pg-sh')}
<defs>${lin('pg-tr', [[0, '#d6f8f3', .75], [.5, '#6fd0c2', .55], [1, '#1d8a7c', .85]], 1, 1)}</defs>
<rect x="24" y="24" width="208" height="204" rx="26" fill="url(#pg-tr)" stroke="#11574e" stroke-width="3"/>
${rim('M48 28 H208 Q228 28 228 48 V204 Q228 224 208 224 H48 Q28 224 28 204 V48 Q28 28 48 28 Z', 2.5, .75)}
${[['red', 'blue', 'green'], ['yellow', 'purple', 'orange'], ['teal', 'blue', 'red']].map((row, r) => row.map((c, k) => key(`pg-${r}${k}`, 42 + k * 60, 42 + r * 60, 52, 52, C[c], 12)).join('')).join('')}
${shine('M28 28 L228 28 L228 92 Q128 124 28 104 Z', 'pg-gl', .45)}`;

// Programs, other takes (the owner, 2026-10-30: "So far I dont love Programs", the tile grid above).
const mini = (n, x, y, s) => `<g transform="translate(${x} ${y}) scale(${s})">${ICONS[n]}</g>`;
const win = (p, x, y, w, h, c, inner = '') => `<defs>${lin(p + '-tb', [[0, c[0]], [.5, c[1]], [1, c[2]]])}${lin(p + '-bd', [[0, '#ffffff', .95], [1, '#d8e2ec', .92]])}</defs>
<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="10" fill="url(#${p}-bd)" stroke="${c[2]}" stroke-width="3" filter="url(#fx-paper)"/>
<path d="M${x} ${y + 10} Q${x} ${y} ${x + 10} ${y} H${x + w - 10} Q${x + w} ${y} ${x + w} ${y + 10} V${y + 30} H${x} Z" fill="url(#${p}-tb)" stroke="${c[2]}" stroke-width="3" filter="url(#fx-small)"/>
<rect x="${x + 4}" y="${y + 3}" width="${w - 8}" height="12" rx="6" fill="#fff" opacity=".55"/>
${[0, 1, 2].map(i => `<circle cx="${x + w - 16 - i * 16}" cy="${y + 15}" r="5" fill="#fff" opacity=".8"/>`).join('')}${inner}`;
ICONS['Programs: windows'] = `
${shadow(132, 230, 104, 13, 'pw-sh')}
${win('pw1', 20, 22, 150, 110, C.green)}
${win('pw2', 56, 62, 150, 110, C.orange)}
${win('pw3', 92, 102, 148, 116, C.blue, `${lines(106, 146, 96, 3, 18)}`)}
${shine('M94 134 H238 V170 Q170 186 94 176 Z', 'pw-gl', .4)}`;
ICONS['Programs: folder'] = folder('pf', `${place(ICONS.Games, 80, 80, .38)}${place(ICONS.Calculator, 128, 66, .38)}${place(ICONS.Browser, 176, 82, .34)}`);
// The owner's pick (2026-10-30): "lets use Program folder".
ICONS.Programs = ICONS['Programs: folder'];
ICONS['Programs: box'] = `
${shadow(128, 230, 110, 14, 'px-sh')}
<defs>
${lin('px-bk', [[0, '#a8ece2', .95], [1, '#18786c', .95]])}
${lin('px-fr', [[0, '#d6f8f3', .7], [.45, '#6fd0c2', .55], [1, '#1d8a7c', .88]])}
${lin('px-fl', [[0, '#e4fbf8', .8], [1, '#3fb3a4', .8]])}
</defs>
<path d="M44 92 L212 92 L236 66 L68 66 Z" fill="url(#px-bk)" stroke="#11574e" stroke-width="3" stroke-linejoin="round"/>
${mini('Games', 30, 20, .48)}${mini('Browser', 118, 2, .46)}${mini('Calculator', 152, 34, .4)}
<path d="M44 92 L4 60 L28 44 L68 66 Z" fill="url(#px-fl)" stroke="#11574e" stroke-width="3" stroke-linejoin="round"/>
<path d="M212 92 L252 66 L236 40 L236 66 Z" fill="url(#px-fl)" stroke="#11574e" stroke-width="3" stroke-linejoin="round"/>
<path d="M44 92 L212 92 L212 220 L44 220 Z" fill="url(#px-fr)" stroke="#11574e" stroke-width="3"/>
<path d="M212 92 L236 66 L236 194 L212 220 Z" fill="url(#px-bk)" stroke="#11574e" stroke-width="3" stroke-linejoin="round" opacity=".9"/>
${rim('M50 98 L206 98 L206 214 L50 214 Z', 2.5, .75)}
${shine('M47 95 L209 95 L209 128 Q128 142 47 130 Z', 'px-gl', .85)}
${glow('M48 194 L208 194 L208 216 L48 216 Z', 'px-gw', .5)}
${streak(186, 104, 92, 6)}`;
