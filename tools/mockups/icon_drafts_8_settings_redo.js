// Settings redraws (the owner, 2026-10-30: "dont love theme and desktop, dont love Storage").
// Three takes each; the palette and the stacked drums stay in batch 5 for comparison.

// Themes & desktop
ICONS['Themes: swatches'] = `
${shadow(128, 230, 100, 13, 'tw-sh')}
${[['purple', -30], ['orange', -10], ['blue', 10], ['teal', 30]].map(([c, r], i) => `<defs>${lin('tw-' + i, [[0, C[c][0]], [.45, C[c][1]], [1, C[c][2]]], 1, 1)}</defs>
<g transform="rotate(${r} 128 206)"><rect x="98" y="20" width="60" height="196" rx="12" fill="url(#tw-${i})" stroke="${C[c][2]}" stroke-width="3"/>
<rect x="104" y="150" width="48" height="56" rx="6" fill="#fbfdff" opacity=".92"/><rect x="110" y="162" width="36" height="7" rx="3" fill="#a9b4c0"/><rect x="110" y="176" width="24" height="7" rx="3" fill="#c6ced8"/>
<rect x="102" y="24" width="16" height="120" rx="8" fill="#fff" opacity=".45"/></g>`).join('')}
<circle cx="128" cy="206" r="13" fill="#d4dbe3" stroke="#3a4250" stroke-width="3"/><circle cx="125" cy="202" r="4" fill="#fff"/>`;

ICONS['Themes: roller'] = `
${shadow(128, 228, 104, 12, 'tr-sh')}
<defs>
${lin('tr-bz', [[0, '#7c8696'], [.25, '#2c323c'], [1, '#0e1116']])}
${lin('tr-old', [[0, '#5aa8ec'], [1, '#bfe2ff']])}
${lin('tr-new', [[0, '#c9f4ee'], [.5, '#3fb3a4'], [1, '#11574e']])}
${lin('tr-rl', [[0, '#ffe0b0'], [.5, '#f58020'], [1, '#a03c02']])}
${lin('tr-mt', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']], 1, 0)}
</defs>
<rect x="12" y="34" width="200" height="166" rx="14" fill="url(#tr-bz)" stroke="#05060a" stroke-width="3"/>
<rect x="24" y="46" width="176" height="130" rx="3" fill="url(#tr-old)"/>
<rect x="24" y="46" width="176" height="74" fill="url(#tr-new)"/>
<path d="M24 46 H200 V66 L24 140 Z" fill="#fff" opacity=".3"/>
<rect x="24" y="184" width="176" height="8" rx="4" fill="#5c6672"/>
<path d="M186 120 H232 V172 H214 V184" fill="none" stroke="#2a3038" stroke-width="12" stroke-linejoin="round"/>
<path d="M186 120 H232 V172 H214 V184" fill="none" stroke="url(#tr-mt)" stroke-width="7" stroke-linejoin="round"/>
<rect x="205" y="180" width="18" height="56" rx="8" fill="url(#tr-rl)" stroke="#7a2c02" stroke-width="2.5"/>
<rect x="209" y="184" width="6" height="46" rx="3" fill="#fff" opacity=".5"/>
<rect x="30" y="102" width="160" height="38" rx="17" fill="url(#tr-new)" stroke="#0e4a42" stroke-width="3"/>
<rect x="38" y="106" width="144" height="12" rx="6" fill="#fff" opacity=".55"/>`;

ICONS['Themes: colour wheel'] = `
${shadow(128, 228, 92, 13, 'tc-sh')}
${(() => { const cs = ['teal', 'blue', 'purple', 'red', 'orange', 'yellow', 'green']; return cs.map((c, i) => { const a0 = (i / cs.length) * 2 * Math.PI - Math.PI / 2, a1 = ((i + 1) / cs.length) * 2 * Math.PI - Math.PI / 2, P = (r, a) => `${(128 + r * Math.cos(a)).toFixed(1)} ${(122 + r * Math.sin(a)).toFixed(1)}`; return `<defs>${rad('tc-' + i, [[0, C[c][0]], [.6, C[c][1]], [1, C[c][2]]], .5, .5, .6)}</defs><path d="M128 122 L${P(94, a0)} A94 94 0 0 1 ${P(94, a1)} Z" fill="url(#tc-${i})" stroke="#fff" stroke-width="3"/>`; }).join(''); })()}
<circle cx="128" cy="122" r="94" fill="none" stroke="#2a3038" stroke-width="3"/>
${orb('tc-c', 128, 122, 30, C.silver, .7)}
<ellipse cx="108" cy="64" rx="60" ry="28" fill="#fff" opacity=".45" transform="rotate(-16 108 64)"/>`;

// Storage
ICONS['Storage: gauge'] = `
${shadow(128, 228, 92, 13, 'sv-sh')}
<defs>${lin('sv-u', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']], 1, 1)}${lin('sv-f', [[0, '#e4fbf8', .8], [1, '#3fb3a4', .7]], 1, 1)}</defs>
<circle cx="128" cy="122" r="74" fill="none" stroke="#0e4a42" stroke-width="44"/>
<circle cx="128" cy="122" r="74" fill="none" stroke="url(#sv-f)" stroke-width="38"/>
<path d="M128 48 A74 74 0 1 1 57.6 144.9" fill="none" stroke="url(#sv-u)" stroke-width="38"/>
<path d="M128 48 A74 74 0 1 1 57.6 144.9" fill="none" stroke="#0a2a60" stroke-width="40" opacity="0"/>
<path d="M76 70 A74 74 0 0 1 180 70" fill="none" stroke="#fff" stroke-width="8" stroke-linecap="round" opacity=".55"/>
<g transform="translate(128 122) scale(.36) translate(-128 -128)">${ICONS['Card inserted'].replace(/ci-/g, 'svc-')}</g>`;

ICONS['Storage: drawers'] = `
${shadow(128, 232, 92, 12, 'sw-sh')}
<defs>${lin('sw-bd', [[0, '#d6f8f3', .8], [.45, '#6fd0c2', .65], [1, '#1d8a7c', .9]], 1, 1)}${lin('sw-dr', [[0, '#e4fbf8', .9], [1, '#3fb3a4', .85]])}${lin('sw-hd', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']])}</defs>
<rect x="44" y="18" width="168" height="210" rx="12" fill="url(#sw-bd)" stroke="#0e4a42" stroke-width="3"/>
${rim('M56 21 H200 Q209 21 209 30', 2.5, .7)}
${[30, 98].map(y => `<rect x="56" y="${y}" width="144" height="58" rx="6" fill="url(#sw-dr)" stroke="#0e4a42" stroke-width="2.5"/><rect x="108" y="${y + 20}" width="40" height="14" rx="7" fill="url(#sw-hd)" stroke="#3a4250" stroke-width="2"/>`).join('')}
${sheetIn(78, 124, -4, 100, 60)}
<path d="M38 166 H218 L210 230 H46 Z" fill="url(#sw-dr)" stroke="#0e4a42" stroke-width="3" stroke-linejoin="round"/>
<rect x="108" y="190" width="40" height="14" rx="7" fill="url(#sw-hd)" stroke="#3a4250" stroke-width="2"/>
${shine('M47 21 H209 V60 Q128 76 47 66 Z', 'sw-gl', .6)}
${glow('M46 208 H210 V228 H46 Z', 'sw-gw', .45)}`;

ICONS['Storage: card and bar'] = `
${shadow(128, 232, 104, 12, 'sb2-sh')}
<g transform="rotate(-8 128 110) translate(0 -2) scale(.82) translate(28 0)">${ICONS['Card inserted'].replace(/ci-/g, 'sbc-').replace(/<ellipse[^>]*>/, '')}</g>
<defs>${lin('sb2-gl', [[0, '#ffffff', .9], [1, '#a9b4c0', .8]])}</defs>
<rect x="18" y="176" width="220" height="44" rx="14" fill="url(#sb2-gl)" stroke="#2a3038" stroke-width="4"/>
${[['blue', 26, 70], ['green', 96, 44], ['orange', 140, 30]].map(([c, x, w], i) => `<defs>${lin('sb2-' + i, [[0, C[c][0]], [.5, C[c][1]], [1, C[c][2]]])}</defs>${i == 0 ? `<path d="M${x + 8} 184 H${x + w} V212 H${x + 8} Q${x} 212 ${x} 204 V192 Q${x} 184 ${x + 8} 184 Z" fill="url(#sb2-${i})"/>` : `<rect x="${x}" y="184" width="${w}" height="28" fill="url(#sb2-${i})"/>`}`).join('')}
<rect x="24" y="182" width="208" height="12" rx="6" fill="#fff" opacity=".5"/>`;
// The owner's pick (2026-10-30), the blue end squared off against the green.
ICONS.Storage = ICONS['Storage: card and bar'];
