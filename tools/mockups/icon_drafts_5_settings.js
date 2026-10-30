// Icon drafts, batch 5: the Settings applets (icon-brief.md "The full set").

const rays = (cx, cy, r0, r1, n = 12, fill = '#ffd23a', stroke = '#c07a04') => `<g fill="${fill}" stroke="${stroke}" stroke-width="2" stroke-linejoin="round">${[...Array(n)].map((_, i) => { const a = i * 2 * Math.PI / n, b = Math.PI / n * .45, P = (r, t) => `${(cx + r * Math.cos(t)).toFixed(1)} ${(cy + r * Math.sin(t)).toFixed(1)}`; return `<path d="M${P(r0, a - b)} L${P(r1, a)} L${P(r0, a + b)} Z"/>`; }).join('')}</g>`;
const sunAt = (p, cx, cy, r) => `${rays(cx, cy, r * 1.05, r * 1.55)}${orb(p, cx, cy, r, ['#fffbe0', '#ffd23a', '#d27a04'], .5)}`;
const person = (p, cx, cy, s = 1, ramp = C.blue) => `<defs>${rad(p, [[0, ramp[0]], [.5, ramp[1]], [1, ramp[2]]], .38, .3, .8)}</defs>
<path d="M${cx - 50 * s} ${cy + 74 * s} Q${cx - 50 * s} ${cy + 22 * s} ${cx} ${cy + 22 * s} Q${cx + 50 * s} ${cy + 22 * s} ${cx + 50 * s} ${cy + 74 * s} Z" fill="url(#${p})" stroke="${ramp[2]}" stroke-width="${2.5 * s}"/>
<circle cx="${cx}" cy="${cy - 18 * s}" r="${32 * s}" fill="url(#${p})" stroke="${ramp[2]}" stroke-width="${2.5 * s}"/>
<ellipse cx="${cx - 8 * s}" cy="${cy - 34 * s}" rx="${18 * s}" ry="${10 * s}" fill="#fff" opacity=".6"/><ellipse cx="${cx - 12 * s}" cy="${cy + 36 * s}" rx="${22 * s}" ry="${7 * s}" fill="#fff" opacity=".4"/>`;
const cloud = (p, t = '') => `<defs>${lin(p, [[0, '#ffffff'], [.6, '#eef4fa'], [1, '#a9bed4']])}</defs>
<g ${t}><path d="M58 206 Q24 206 24 174 Q24 144 56 142 Q60 106 96 102 Q124 100 138 124 Q150 110 172 116 Q198 124 198 152 Q226 156 226 180 Q226 206 198 206 Z" fill="url(#${p})" stroke="#6f8aa6" stroke-width="3" stroke-linejoin="round"/>
<path d="M30 168 Q32 148 58 148 Q64 112 96 108 Q122 108 134 132 Q150 116 170 122 Q192 130 192 156 Q120 176 30 168 Z" fill="#fff" opacity=".6"/></g>`;

ICONS['Display & brightness'] = `
${shadow(128, 228, 100, 12, 'sd-sh')}
<defs>${lin('sd-bz', [[0, '#7c8696'], [.25, '#2c323c'], [1, '#0e1116']])}${lin('sd-sc', [[0, '#7cc6ff'], [1, '#0c4aa0']])}</defs>
<rect x="18" y="30" width="180" height="150" rx="14" fill="url(#sd-bz)" stroke="#05060a" stroke-width="3"/>
<rect x="30" y="42" width="156" height="114" rx="3" fill="url(#sd-sc)"/>
<path d="M30 42 H186 V64 L30 128 Z" fill="#fff" opacity=".3"/>
<rect x="30" y="164" width="156" height="8" rx="4" fill="#5c6672"/><rect x="30" y="164" width="110" height="8" rx="4" fill="#ffd23a"/>
${sunAt('sd-sun', 186, 168, 36)}`;

ICONS.Sound = `
${shadow(120, 228, 94, 12, 'sn-sh')}
<defs>${lin('sn-bx', [[0, '#9aa6b6'], [.3, '#3c4450'], [1, '#14181e']], 1, 0)}${lin('sn-w', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
<path d="M24 92 H62 L120 40 V208 L62 156 H24 Z" fill="url(#sn-bx)" stroke="#05060a" stroke-width="3" stroke-linejoin="round"/>
<path d="M28 96 H62 L116 48 V96 Q70 120 28 116 Z" fill="#fff" opacity=".35"/>
${[44, 80, 116].map((r, i) => { const P = a => `${(116 + r * Math.cos(a * Math.PI / 180)).toFixed(1)} ${(124 + r * Math.sin(a * Math.PI / 180)).toFixed(1)}`; return `<path d="M${P(-48)} A${r} ${r} 0 0 1 ${P(48)}" fill="none" stroke="#0a2a60" stroke-width="20" stroke-linecap="round" opacity="${1 - i * .2}"/><path d="M${P(-48)} A${r} ${r} 0 0 1 ${P(48)}" fill="none" stroke="url(#sn-w)" stroke-width="14" stroke-linecap="round" opacity="${1 - i * .2}"/>`; }).join('')}`;

ICONS.Buttons = `
${shadow(128, 228, 96, 13, 'sb-sh')}
<defs>${rad('sb-rk', [[0, '#ffffff'], [.6, '#c4ccd6'], [1, '#5c6672']], .4, .32, .75)}${rad('sb-cn', [[0, '#e0f4ff'], [.5, '#3a92ea'], [1, '#0b3a80']], .4, .3, .75)}</defs>
<circle cx="128" cy="122" r="98" fill="url(#sb-rk)" stroke="#3a4250" stroke-width="3"/>
<circle cx="128" cy="122" r="92" fill="none" stroke="#fff" stroke-width="3" opacity=".7"/>
<g fill="#4c5664">${[0, 90, 180, 270].map(r => `<path d="M128 40 L146 64 L110 64 Z" transform="rotate(${r} 128 122)"/>`).join('')}</g>
<circle cx="128" cy="122" r="38" fill="url(#sb-cn)" stroke="#0a2a60" stroke-width="3"/>
<ellipse cx="122" cy="106" rx="22" ry="11" fill="#fff" opacity=".6"/>
<ellipse cx="104" cy="58" rx="56" ry="24" fill="#fff" opacity=".4" transform="rotate(-16 104 58)"/>`;

ICONS['Power & battery'] = `
${shadow(128, 232, 70, 11, 'sp-sh')}
<defs>${lin('sp-gl', [[0, '#ffffff', .85], [.5, '#d8e2ec', .5], [1, '#8592a2', .85]], 1, 0)}${lin('sp-fl', [[0, '#d6fab8'], [.5, '#36b02a'], [1, '#126810']], 1, 0)}${lin('sp-cp', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']], 1, 0)}${lin('sp-bl', [[0, '#fff6c0'], [1, '#f2a01c']])}</defs>
<rect x="104" y="14" width="48" height="22" rx="5" fill="url(#sp-cp)" stroke="#3a4250" stroke-width="2.5"/>
<rect x="66" y="30" width="124" height="198" rx="18" fill="url(#sp-gl)" stroke="#3a4250" stroke-width="3"/>
<rect x="76" y="90" width="104" height="128" rx="10" fill="url(#sp-fl)" stroke="#126810" stroke-width="2"/>
<path d="M140 52 L96 140 H126 L112 204 L162 108 H130 Z" fill="url(#sp-bl)" stroke="#8a5a04" stroke-width="3" stroke-linejoin="round"/>
<rect x="76" y="36" width="20" height="184" rx="10" fill="#fff" opacity=".5"/>`;

// Memory means RAM; flash and cards are under Storage.
ICONS['Memory settings'] = ICONS.RAM;

// The first take, stacked drums (not liked by the owner).
ICONS['Storage: drums'] = `
${shadow(128, 226, 104, 13, 'sg-sh')}
<defs>${lin('sg-tp', [[0, '#e4fbf8'], [1, '#3fb3a4']])}${lin('sg-fr', [[0, '#e6ebf0'], [.5, '#a9b4c0'], [1, '#4c5664']])}</defs>
${[0, 1, 2].map(i => { const y = 150 - i * 50; return `<path d="M30 ${y} V${y + 40} Q30 ${y + 62} 128 ${y + 62} Q226 ${y + 62} 226 ${y + 40} V${y} Z" fill="url(#sg-fr)" stroke="#3a4250" stroke-width="3"/><ellipse cx="128" cy="${y}" rx="98" ry="22" fill="url(#sg-tp)" stroke="#11574e" stroke-width="3"/><ellipse cx="108" cy="${y - 6}" rx="56" ry="9" fill="#fff" opacity=".55"/><circle cx="196" cy="${y + 40}" r="5" fill="#36e02a" stroke="#126810"/>`; }).join('')}`;

ICONS['Clock & region'] = `
${shadow(120, 228, 92, 13, 'sr-sh')}
<g transform="translate(6 4) scale(.86)">${ICONS.Browser.replace(/br-/g, 'srg-').replace(/<ellipse cx="128" cy="228"[^>]*>/, '')}</g>
<circle cx="190" cy="180" r="48" fill="#fff"/>
<defs>${rad('sr-fc', [[0, '#ffffff'], [.85, '#eef3f8'], [1, '#c2ccd8']], .45, .4, .6)}</defs>
<circle cx="190" cy="180" r="42" fill="url(#sr-fc)" stroke="#0b3a80" stroke-width="6"/>
<g stroke="#3a4250" stroke-linecap="round">${[0, 3, 6, 9].map(i => { const a = i * Math.PI / 2; return `<path d="M${(190 + 30 * Math.sin(a)).toFixed(1)} ${(180 - 30 * Math.cos(a)).toFixed(1)} L${(190 + 36 * Math.sin(a)).toFixed(1)} ${(180 - 36 * Math.cos(a)).toFixed(1)}" stroke-width="4"/>`; }).join('')}</g>
<path d="M190 180 V154 M190 180 L172 190" stroke="#1c2026" stroke-width="6" stroke-linecap="round"/>
<ellipse cx="180" cy="160" rx="22" ry="10" fill="#fff" opacity=".5"/>`;

ICONS['Owner info'] = `
${shadow(128, 228, 96, 13, 'so2-sh')}
${person('so2-p', 112, 96, 1.5)}
<g transform="rotate(-6 186 182)"><rect x="140" y="152" width="96" height="60" rx="8" fill="#fbfdff" stroke="#6d7886" stroke-width="2.5"/><rect x="140" y="152" width="96" height="18" rx="8" fill="#e3321e"/><rect x="140" y="162" width="96" height="8" fill="#e3321e"/>${lines(152, 180, 72, 2, 14)}</g>`;

// The first take, a palette (not liked by the owner).
ICONS['Themes: palette'] = `
${shadow(128, 226, 104, 13, 'sm-sh')}
<defs>${lin('sm-pl', [[0, '#fff2d8'], [.6, '#e8c890'], [1, '#a87a3a']], 1, 1)}${lin('sm-br', [[0, '#ffe0a0'], [.5, '#c07a10'], [1, '#6a3a04']], 1, 0)}</defs>
<path d="M128 30 Q228 30 232 116 Q236 172 188 168 Q160 166 164 192 Q168 222 128 222 Q24 222 24 126 Q24 30 128 30 Z" fill="url(#sm-pl)" stroke="#6a4a1a" stroke-width="3"/>
<ellipse cx="150" cy="186" rx="14" ry="11" fill="#6a4a1a" opacity=".35"/>
${[['red', 74, 92], ['yellow', 120, 66], ['green', 172, 80], ['blue', 196, 124], ['purple', 66, 146], ['teal', 104, 184]].map(([c, x, y], i) => orb('sm-' + i, x, y, 19, C[c], .55)).join('')}
<path d="M40 112 Q40 40 128 36 Q190 36 214 80 Q130 64 40 112 Z" fill="#fff" opacity=".4"/>
<g transform="rotate(-38 160 140)"><rect x="152" y="40" width="16" height="120" rx="7" fill="url(#sm-br)" stroke="#5a2a04" stroke-width="2"/><rect x="150" y="150" width="20" height="16" fill="#c4ccd6" stroke="#4c5664" stroke-width="2"/><path d="M150 166 Q148 196 160 214 Q172 196 170 166 Z" fill="#2a7fe0" stroke="#0b3a80" stroke-width="2"/></g>`;

ICONS['Light & dark'] = `
${shadow(128, 226, 92, 13, 'sl-sh')}
<defs><clipPath id="sl-l"><rect x="0" y="0" width="128" height="256"/></clipPath><clipPath id="sl-r"><rect x="128" y="0" width="128" height="256"/></clipPath>
${lin('sl-day', [[0, '#7cc6ff'], [1, '#d8f0ff']])}${lin('sl-nt', [[0, '#0b1a40'], [1, '#2a3f7a']])}</defs>
<circle cx="128" cy="122" r="96" fill="url(#sl-day)" clip-path="url(#sl-l)"/>
<circle cx="128" cy="122" r="96" fill="url(#sl-nt)" clip-path="url(#sl-r)"/>
<g clip-path="url(#sl-l)">${sunAt('sl-sun', 82, 122, 25)}</g>
<g clip-path="url(#sl-r)"><mask id="sl-m"><rect x="128" y="0" width="128" height="256" fill="#fff"/><circle cx="190" cy="108" r="30" fill="#000"/></mask>
<circle cx="172" cy="122" r="34" fill="#fff4c8" stroke="#c0a040" stroke-width="2" mask="url(#sl-m)"/>
${[[154, 70], [204, 104], [200, 64], [160, 190]].map(([x, y]) => `<circle cx="${x}" cy="${y}" r="3" fill="#fff"/>`).join('')}</g>
<circle cx="128" cy="122" r="96" fill="none" stroke="#0a2a60" stroke-width="4"/>
<path d="M128 26 V218" stroke="#fff" stroke-width="3" opacity=".7"/>
<ellipse cx="108" cy="66" rx="60" ry="28" fill="#fff" opacity=".45" transform="rotate(-14 108 66)"/>`;

ICONS['Touch alignment'] = `
${shadow(128, 228, 96, 13, 'st2-sh')}
<circle cx="120" cy="130" r="90" fill="#fbfdff" stroke="#6d7886" stroke-width="3"/>
${[70, 48, 26].map((r, i) => `<circle cx="120" cy="130" r="${r}" fill="${['#fff', '#e3321e', '#fff'][i]}" stroke="#e3321e" stroke-width="10"/>`).join('')}
<circle cx="120" cy="130" r="8" fill="#e3321e"/>
<path d="M120 30 V230 M20 130 H220" stroke="#3a4250" stroke-width="2" opacity=".6"/>
<ellipse cx="96" cy="72" rx="56" ry="24" fill="#fff" opacity=".45" transform="rotate(-16 96 72)"/>
<defs>${lin('st2-sy', [[0, '#e6ebf0'], [.5, '#8592a2'], [1, '#3c4450']], 1, 0)}</defs>
<g transform="rotate(40 120 130)"><rect x="112" y="-28" width="16" height="140" rx="7" fill="url(#st2-sy)" stroke="#2a3038" stroke-width="2"/><path d="M112 112 L120 132 L128 112 Z" fill="#2a3038"/></g>`;

ICONS['Keyboard & input'] = `
${shadow(128, 214, 112, 13, 'sk-sh')}
<defs>${lin('sk-bd', [[0, '#9aa6b6'], [.3, '#3c4450'], [1, '#14181e']])}</defs>
<path d="M30 92 L226 92 L246 196 Q248 206 238 206 L18 206 Q8 206 10 196 Z" fill="url(#sk-bd)" stroke="#05060a" stroke-width="3" stroke-linejoin="round"/>
${(() => {
  // Keys follow the body's slanted sides (x 30..226 at y 92, 10..246 at y 196), inset 12, rows centred.
  const L = y => 30 - 20 * (y - 92) / 104, R = y => 226 + 20 * (y - 92) / 104, g = 4;
  return [[100, 10], [126, 10], [152, 9]].map(([y, n], r) => {
    const x0 = L(y + 22) + 12, x1 = R(y + 22) - 12, w = (x1 - x0 - (n - 1) * g) / n;
    return [...Array(n)].map((_, c) => key(`sk-${r}-${c}`, x0 + c * (w + g), y, w, 22, C.silver, 4)).join('');
  }).join('') + key('sk-sp', 128 - 64, 178, 128, 12, C.silver, 4);
})()}
<path d="M30 94 L226 94 L232 120 Q128 132 24 122 Z" fill="#fff" opacity=".25"/>`;

ICONS.Wireless = `<g transform="translate(-20 -16) scale(.92)">${ICONS.Network.replace(/nw-/g, 'swn-')}</g>
<rect x="164" y="128" width="64" height="100" rx="32" fill="#fff"/>
<defs>${lin('swb', [[0, '#c6ecff'], [.45, '#2a7fe0'], [1, '#0b3a80']], 1, 1)}</defs>
<rect x="168" y="132" width="56" height="92" rx="28" fill="url(#swb)" stroke="#0a2a60" stroke-width="2.5"/>
<path d="M184 162 L210 188 L196 204 L196 148 L210 162 L184 190" fill="none" stroke="#fff" stroke-width="6" stroke-linejoin="round" stroke-linecap="round"/>
<ellipse cx="190" cy="146" rx="16" ry="8" fill="#fff" opacity=".5"/>`;

ICONS['Security & keys'] = `
${shadow(124, 232, 86, 12, 'sy-sh')}
<defs>${lin('sy-sd', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']], 1, 1)}${lin('sy-k', [[0, '#fff3b0'], [.5, '#e8b020'], [1, '#8a5a04']], 1, 1)}</defs>
<path d="M118 16 Q160 36 206 34 Q210 130 186 172 Q160 214 118 232 Q76 214 50 172 Q26 130 30 34 Q76 36 118 16 Z" fill="url(#sy-sd)" stroke="#0a2a60" stroke-width="3" stroke-linejoin="round"/>
${rim('M118 24 Q158 42 198 42 Q200 128 178 166 Q156 204 118 222', 2.5, .55)}
<path d="M34 40 Q76 42 118 22 Q160 42 202 40 Q204 76 200 96 Q118 130 36 100 Q32 76 34 40 Z" fill="#fff" opacity=".4"/>
<g transform="rotate(-40 160 170)"><circle cx="160" cy="130" r="30" fill="url(#sy-k)" stroke="#6a4204" stroke-width="3"/><circle cx="160" cy="124" r="10" fill="#0b3a80" stroke="#6a4204" stroke-width="2"/>
<rect x="152" y="156" width="16" height="74" rx="4" fill="url(#sy-k)" stroke="#6a4204" stroke-width="2.5"/><rect x="168" y="196" width="18" height="10" fill="url(#sy-k)" stroke="#6a4204" stroke-width="2"/><rect x="168" y="214" width="12" height="10" fill="url(#sy-k)" stroke="#6a4204" stroke-width="2"/>
<ellipse cx="152" cy="118" rx="12" ry="6" fill="#fff" opacity=".6"/></g>`;

ICONS['Sources & accounts'] = `
${shadow(128, 226, 100, 13, 'sa-sh')}
${cloud('sa-c', 'transform="translate(-14 -26)"')}
<circle cx="184" cy="168" r="50" fill="#fff" opacity=".9"/>
${orb('sa-o', 184, 168, 46, C.blue, 0)}
${person('sa-p', 184, 168, .62, ['#ffffff', '#e6f2ff', '#8fb6e8'])}`;

ICONS.Packs = `
${shadow(128, 228, 96, 13, 'sk2-sh')}
<defs>${lin('sk2-a', [[0, '#c9f4ee'], [.5, '#3fb3a4'], [1, '#11574e']], 1, 1)}${lin('sk2-b', [[0, '#ffe0b0'], [.5, '#f58020'], [1, '#a03c02']], 1, 1)}</defs>
<path d="M30 40 H90 Q84 20 104 18 Q124 20 118 40 H178 V100 Q198 94 200 114 Q198 134 178 128 V188 H118 Q124 208 104 210 Q84 208 90 188 H30 Z" fill="url(#sk2-a)" stroke="#0e4a42" stroke-width="3" stroke-linejoin="round" transform="rotate(-8 104 114)"/>
<path d="M34 44 H88 Q84 26 104 24 Q124 26 120 44 H174 V90 Q120 112 34 96 Z" fill="#fff" opacity=".45" transform="rotate(-8 104 114)"/>
<path d="M140 128 H180 Q176 112 192 110 Q208 112 204 128 H236 V160 Q250 156 250 172 Q250 188 236 184 V228 H140 V196 Q154 200 154 184 Q154 168 140 172 Z" fill="url(#sk2-b)" stroke="#7a2c02" stroke-width="3" stroke-linejoin="round"/>
<path d="M144 132 H178 Q176 116 192 114 Q206 116 206 132 H232 V160 Q186 180 144 168 Z" fill="#fff" opacity=".45"/>`;

ICONS.About = `<g transform="translate(-14 0)">${ICONS['My Device'].replace(/dv-/g, 'sab-')}</g>${badge('info', 'sab-i', 186, 182, 46)}`;

ICONS.Updates = `
${shadow(128, 228, 92, 13, 'su-sh')}
${orb('su-o', 128, 122, 94, C.green, .4)}
${arcArrow(128, 122, 62, 140, 400, '#fff', 20, 1.1)}
<path d="M128 72 L162 112 H142 V160 H114 V112 H94 Z" fill="#fff" stroke="#fff" stroke-width="4" stroke-linejoin="round"/>`;
