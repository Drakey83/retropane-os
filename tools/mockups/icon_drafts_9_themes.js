// Themes & desktop, a new round (the owner, 2026-10-30: "still dont like any of the themes";
// not liked so far: the palette, swatches, roller, colour wheel).

const scene = (p, x, y, w, h, r, night) => `<defs>${lin(p + '-s', night ? [[0, '#0b1a40'], [1, '#3a5a9a']] : [[0, '#5aa8ec'], [1, '#d8f0ff']])}${lin(p + '-h', night ? [[0, '#1d5a50'], [1, '#08241f']] : [[0, '#8fe07a'], [1, '#2a8a20']])}</defs>
<g transform="rotate(${r} ${x + w / 2} ${y + h / 2})">
<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="8" fill="#fbfdff" stroke="#6d7886" stroke-width="3"/>
<rect x="${x + 10}" y="${y + 10}" width="${w - 20}" height="${h - 20}" rx="2" fill="url(#${p}-s)"/>
${night ? `<path d="M${x + w - 40} ${y + 22} a14 14 0 1 0 12 22 a11 11 0 1 1 -12 -22 Z" fill="#fff4c8"/>${[[.2, .25], [.4, .18], [.55, .35]].map(([a, b]) => `<circle cx="${x + w * a}" cy="${y + h * b}" r="2.5" fill="#fff"/>`).join('')}` : `<circle cx="${x + w - 34}" cy="${y + 32}" r="13" fill="#ffe27a"/>`}
<path d="M${x + 10} ${y + h - 30} Q${x + w * .35} ${y + h * .45} ${x + w * .55} ${y + h - 40} Q${x + w * .75} ${y + h * .5} ${x + w - 10} ${y + h - 36} V${y + h - 10} H${x + 10} Z" fill="url(#${p}-h)"/>
<path d="M${x + 4} ${y + 4} H${x + w - 4} V${y + h * .35} Q${x + w / 2} ${y + h * .5} ${x + 4} ${y + h * .42} Z" fill="#fff" opacity=".35"/></g>`;

ICONS['Themes: wallpaper swap'] = `
${shadow(132, 230, 104, 13, 'ts-sh')}
${scene('ts-n', 18, 26, 150, 110, -8, true)}
${scene('ts-d', 82, 96, 156, 116, 4, false)}
<defs>${lin('ts-a', [[0, '#ffe0b0'], [.5, '#f58020'], [1, '#a03c02']])}</defs>
${arcArrow(150, 96, 62, 250, 340, 'url(#ts-a)', 16, 1.2)}`;

ICONS['Themes: prism'] = `
${shadow(128, 222, 100, 12, 'tp-sh')}
<defs>
${lin('tp-g', [[0, '#e4fbf8', .75], [.5, '#76d0c4', .5], [1, '#1d8a7c', .8]], 1, 1)}
${lin('tp-b', [[0, '#fff', 0], [1, '#fff', .95]], 1, 0)}
</defs>
<path d="M4 158 L96 128" stroke="url(#tp-b)" stroke-width="14" stroke-linecap="round"/>
${['#e3321e', '#f58020', '#f2c01c', '#36b02a', '#2a7fe0', '#9a4ae0'].map((c, i) => `<path d="M156 128 L252 ${92 + i * 22} L252 ${114 + i * 22} Z" fill="${c}" opacity=".92"/>`).join('')}
<path d="M128 30 L218 202 Q222 210 212 210 L44 210 Q34 210 38 202 Z" fill="url(#tp-g)" stroke="#0e4a42" stroke-width="3" stroke-linejoin="round"/>
${rim('M128 44 L208 200 L48 200 Z', 2.5, .75)}
<path d="M128 40 L96 102 Q120 110 150 82 Z" fill="#fff" opacity=".65"/>
<path d="M96 128 L156 128" stroke="#fff" stroke-width="6" opacity=".8"/>
${glow('M52 186 L204 186 L210 204 L46 204 Z', 'tp-gw', .5)}`;

const drop = (p, cx, cy, s, ramp) => `<defs>${rad(p, [[0, ramp[0]], [.55, ramp[1]], [1, ramp[2]]], .38, .55, .7)}</defs>
<path d="M${cx} ${cy - s * 1.45} C${cx + s * .25} ${cy - s * .95} ${cx + s} ${cy - s * .35} ${cx + s} ${cy + s * .15} A${s} ${s} 0 1 1 ${cx - s} ${cy + s * .15} C${cx - s} ${cy - s * .35} ${cx - s * .25} ${cy - s * .95} ${cx} ${cy - s * 1.45} Z" fill="url(#${p})" stroke="${ramp[2]}" stroke-width="${(s / 18).toFixed(1)}"/>
<ellipse cx="${cx - s * .38}" cy="${cy - s * .05}" rx="${s * .2}" ry="${s * .42}" fill="#fff" opacity=".75" transform="rotate(18 ${cx - s * .38} ${cy - s * .05})"/>
<ellipse cx="${cx + s * .3}" cy="${cy + s * .62}" rx="${s * .34}" ry="${s * .14}" fill="#fff" opacity=".3"/>`;
ICONS['Themes: paint drops'] = `
${shadow(128, 228, 104, 12, 'td2-sh')}
${drop('td2-o', 62, 158, 42, C.orange)}
${drop('td2-b', 196, 150, 44, C.blue)}
${drop('td2-t', 128, 126, 62, C.teal)}`;

ICONS['Themes: paint can'] = `
${shadow(128, 230, 96, 13, 'tk2-sh')}
<defs>
${lin('tk2-cn', [[0, '#8592a2'], [.2, '#ffffff'], [.55, '#c4ccd6'], [1, '#5c6672']], 1, 0)}
${lin('tk2-lb', [[0, '#c9f4ee', .95], [.5, '#3fb3a4', .9], [1, '#0e4a42', .95]])}
${rad('tk2-pt', [[0, '#c9f4ee'], [.6, '#3fb3a4'], [1, '#11574e']], .4, .4, .7)}
${lin('tk2-hd', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']])}
</defs>
<path d="M44 98 Q44 20 128 20 Q212 20 212 98" fill="none" stroke="#3a4250" stroke-width="10"/>
<path d="M44 98 Q44 20 128 20 Q212 20 212 98" fill="none" stroke="url(#tk2-hd)" stroke-width="6"/>
<path d="M48 96 V206 Q48 226 128 226 Q208 226 208 206 V96 Z" fill="url(#tk2-cn)" stroke="#2a3038" stroke-width="3"/>
<path d="M48 124 Q128 142 208 124 V184 Q128 202 48 184 Z" fill="url(#tk2-lb)" stroke="#0e4a42" stroke-width="2"/>
<path d="M88 168 Q104 140 128 156 Q152 172 168 146" fill="none" stroke="#fff" stroke-width="8" stroke-linecap="round"/>
<ellipse cx="128" cy="96" rx="80" ry="22" fill="#c4ccd6" stroke="#2a3038" stroke-width="3"/>
<ellipse cx="128" cy="98" rx="68" ry="16" fill="url(#tk2-pt)"/>
<path d="M62 102 Q60 126 66 138 Q72 146 74 132 Q74 116 80 108 Z" fill="url(#tk2-pt)" stroke="#11574e" stroke-width="1.5"/>
<ellipse cx="108" cy="92" rx="30" ry="6" fill="#fff" opacity=".6"/>
<rect x="60" y="102" width="14" height="112" rx="7" fill="#fff" opacity=".45"/>`;

// The owner's pick (2026-10-30): "Wallpaper swap".
ICONS['Themes & desktop'] = ICONS['Themes: wallpaper swap'];
