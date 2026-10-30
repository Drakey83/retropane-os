// Glass recipe, version 2 (the owner, 2026-10-30, "Yes, try it"): SVG filter effects on
// top of the gradients the first drafts used. Loaded after icon_drafts.js, before the groups.
//  - glassFx: a real bevel (specular light from the top left on the shape's own blurred
//    edge; lit low, at 30 degrees, with a tight highlight, so flat faces stay their own
//    colour and only edges turned to the light shine), plus an inner shade along the bottom and right edges (glass thickness).
//  - dropFx: a soft drop shadow under a whole object.
//  - softFx: a plain blur, for glows, blooms and the light pooled inside glass.

const glassFx = (id, { tint = '#063a34', shade = .5, depth = 6, spec = .8, shine = 40, scale = 4, elev = 30 } = {}) => `<filter id="${id}" x="-15%" y="-15%" width="130%" height="130%" color-interpolation-filters="sRGB">
<feGaussianBlur in="SourceAlpha" stdDeviation="${depth}" result="b"/>
<feOffset in="b" dx="${(-depth * .35).toFixed(1)}" dy="${(-depth * .5).toFixed(1)}" result="bo"/>
<feComposite in="SourceAlpha" in2="bo" operator="arithmetic" k2="1" k3="-1" result="edge"/>
<feFlood flood-color="${tint}" flood-opacity="${shade}"/>
<feComposite in2="edge" operator="in" result="ish"/>
<feGaussianBlur in="SourceAlpha" stdDeviation="${(scale * .75).toFixed(1)}" result="bump"/>
<feSpecularLighting in="bump" surfaceScale="${scale}" specularConstant="${spec}" specularExponent="${shine}" lighting-color="#fff" result="sp"><feDistantLight azimuth="225" elevation="${elev}"/></feSpecularLighting>
<feComposite in="sp" in2="SourceAlpha" operator="in" result="spi"/>
<feMerge result="m"><feMergeNode in="SourceGraphic"/><feMergeNode in="ish"/></feMerge>
<feComposite in="m" in2="spi" operator="arithmetic" k2="1" k3="1"/>
</filter>`;
const dropFx = (id, dy = 7, blur = 5, o = .35) => `<filter id="${id}" x="-20%" y="-20%" width="140%" height="150%">
<feGaussianBlur in="SourceAlpha" stdDeviation="${blur}"/><feOffset dx="${dy * .4}" dy="${dy}" result="o"/>
<feComponentTransfer in="o" result="s"><feFuncA type="linear" slope="${o}"/></feComponentTransfer>
<feMerge><feMergeNode in="s"/><feMergeNode in="SourceGraphic"/></feMerge></filter>`;
const softFx = (id, s) => `<filter id="${id}" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="${s}"/></filter>`;

// --- The three test icons (the artist brief's first step), drawn with the new recipe ---

ICONS['Calendar v2'] = `
<defs>
${dropFx('c2-dr')}
${glassFx('c2-hdf', { tint: '#5a0a02', shade: .5, depth: 5, spec: 1, scale: 3 })}
${glassFx('c2-pgf', { tint: '#2a3a4c', shade: .22, depth: 5, spec: .45, scale: 2 })}
${glassFx('c2-rgf', { tint: '#14181e', shade: .45, depth: 2, spec: 1.2, scale: 2 })}
${softFx('c2-sf', 3)}${softFx('c2-sf1', .8)}
${lin('c2-pg', [[0, '#ffffff'], [.7, '#f2f6fa'], [1, '#d4dde7']])}
${lin('c2-bk', [[0, '#e6ecf2'], [1, '#a9b6c4']])}
${lin('c2-hd', [[0, '#ff8a70'], [.5, '#e3321e'], [1, '#a0180a']])}
${lin('c2-ring', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#5d6874']], 1, 0)}
${lin('c2-gl', [[0, '#fff', .9], [1, '#fff', .15]])}
</defs>
${shadow(128, 232, 100, 14, 'c2-sh')}
<g filter="url(#c2-dr)">
<rect x="46" y="48" width="176" height="178" rx="10" fill="url(#c2-bk)" stroke="#6d7886" stroke-width="2" filter="url(#c2-pgf)"/>
<rect x="40" y="42" width="176" height="178" rx="10" fill="#eef2f6" stroke="#7d8896" stroke-width="2" filter="url(#c2-pgf)"/>
<rect x="34" y="36" width="176" height="178" rx="10" fill="url(#c2-pg)" stroke="#6d7886" stroke-width="2.5" filter="url(#c2-pgf)"/>
<path d="M34 46 Q34 36 44 36 L200 36 Q210 36 210 46 L210 90 L34 90 Z" fill="url(#c2-hd)" stroke="#7a1008" stroke-width="2.5" filter="url(#c2-hdf)"/>
<path d="M39 41 Q39 39 44 39 L200 39 Q206 39 206 44 L206 62 Q122 70 39 62 Z" fill="url(#c2-gl)" filter="url(#c2-sf1)"/>
<ellipse cx="88" cy="56" rx="40" ry="9" fill="#fff" opacity=".35" filter="url(#c2-sf)"/>
<text x="122" y="147.5" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="54" fill="#fff" opacity=".9">30</text>
<text x="122" y="146" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="54" fill="#2b3644">30</text>
<g fill="#c6ced8">${[0, 1, 2, 3, 4, 5].map(i => [0, 1, 2].map(j => `<rect x="${52 + i * 24}" y="${158 + j * 15}" width="17" height="9" rx="2"/>`).join('')).join('')}</g>
<rect x="124" y="173" width="17" height="9" rx="2" fill="#e8402a" filter="url(#c2-hdf)"/>
<g filter="url(#c2-rgf)">${[66, 98, 146, 178].map(x => `<rect x="${x - 6}" y="20" width="12" height="36" rx="6" fill="url(#c2-ring)" stroke="#4a5460"/>`).join('')}</g>
</g>`;

// The teal glass folder, with the new recipe. inner = what stands inside.
const folder2 = (p, inner = '') => `
<defs>
${dropFx(p + '-dr')}
${glassFx(p + '-gb', { tint: '#063a34', shade: .45, depth: 6, spec: .7, scale: 4 })}
${glassFx(p + '-gf', { tint: '#063a34', shade: .5, depth: 9, spec: .95, scale: 5 })}
${softFx(p + '-sf', 7)}${softFx(p + '-sf1', 1.2)}
${lin(p + '-bk', [[0, '#a8ece2', .95], [.5, '#3fb3a4', .9], [1, '#18786c', .95]])}
${lin(p + '-fr', [[0, '#c8f4ee', .72], [.45, '#5cc6b8', .58], [1, '#1d8a7c', .86]])}
${lin(p + '-gl', [[0, '#fff', .92], [1, '#fff', .12]])}
</defs>
${shadow(128, 226, 110, 14, p + '-sh')}
<g filter="url(#${p}-dr)">
<path d="M30 52 Q30 42 40 42 L100 42 Q108 42 112 50 L120 62 L214 62 Q224 62 224 72 L224 206 L30 206 Z" fill="url(#${p}-bk)" stroke="#11574e" stroke-width="3" filter="url(#${p}-gb)"/>
${inner}
<path d="M22 100 Q22 92 30 92 L226 92 Q234 92 233 100 L222 208 Q221 216 213 216 L40 216 Q32 216 31 208 Z" fill="url(#${p}-fr)" stroke="#11574e" stroke-width="3" filter="url(#${p}-gf)"/>
<ellipse cx="128" cy="198" rx="78" ry="13" fill="#eafffb" opacity=".6" filter="url(#${p}-sf)"/>
<path d="M27 97 L229 97 L226 124 Q128 136 29 124 Z" fill="url(#${p}-gl)" filter="url(#${p}-sf1)"/>
<rect x="196" y="104" width="9" height="88" rx="4.5" fill="#fff" opacity=".5" transform="rotate(6 196 104)" filter="url(#${p}-sf1)"/>
</g>`;

ICONS['Music v2'] = folder2('m2', `
<defs>${lin('m2-nt', [[0, '#9ad4ff'], [.5, '#2a7fe0'], [1, '#0c3c88']])}
${glassFx('m2-ntf', { tint: '#06204a', shade: .5, depth: 4, spec: 1.1, scale: 3 })}</defs>
<path d="M118 56 L190 38 L190 150 A22 17 0 1 1 172 134 L172 72 L136 81 L136 168 A22 17 0 1 1 118 152 Z" fill="url(#m2-nt)" stroke="#0a2a60" stroke-width="3" filter="url(#m2-ntf)"/>
<path d="M124 62 L184 47 L184 56 L124 71 Z" fill="#fff" opacity=".6" filter="url(#m2-sf1)"/>`);

ICONS['My Device v2'] = `
<defs>
${dropFx('d2-dr')}
${glassFx('d2-bdf', { tint: '#000', shade: .55, depth: 5, spec: 1, scale: 4 })}
${glassFx('d2-scf', { tint: '#000', shade: .75, depth: 4, spec: 0, scale: 1 })}
${glassFx('d2-btf', { tint: '#1c2026', shade: .4, depth: 3, spec: 1.1, scale: 3 })}
${softFx('d2-sf', 4)}${softFx('d2-sf1', 1)}
${lin('d2-bd', [[0, '#5a6474'], [.3, '#2c323c'], [1, '#14181e']], 1, 1)}
${lin('d2-sc', [[0, '#cdeeff'], [.5, '#3c9cf0'], [1, '#0c4aa0']])}
${lin('d2-hl', [[0, '#7cd87a'], [1, '#2a8a3a']])}
${rad('d2-bt', [[0, '#ffffff'], [.6, '#c4ccd6'], [1, '#6c7886']], .4, .3, .7)}
${lin('d2-rf', [[0, '#fff', .55], [1, '#fff', .05]], 1, 1)}
</defs>
${shadow(128, 236, 72, 12, 'd2-sh')}
<g filter="url(#d2-dr)">
<rect x="66" y="18" width="124" height="208" rx="18" fill="url(#d2-bd)" stroke="#0a0c10" stroke-width="3" filter="url(#d2-bdf)"/>
<g filter="url(#d2-scf)"><rect x="80" y="38" width="96" height="124" rx="3" fill="url(#d2-sc)"/>
<path d="M80 128 Q128 100 176 118 L176 162 L80 162 Z" fill="url(#d2-hl)"/>
<circle cx="150" cy="64" r="12" fill="#ffe27a"/></g>
<path d="M80 38 L176 38 L176 70 L80 118 Z" fill="url(#d2-rf)" filter="url(#d2-sf1)"/>
<circle cx="128" cy="194" r="17" fill="url(#d2-bt)" stroke="#2a3038" stroke-width="2" filter="url(#d2-btf)"/>
${[92, 164].map(x => `<rect x="${x - 10}" y="186" width="20" height="14" rx="4" fill="#5c6672" filter="url(#d2-btf)"/>`).join('')}
<rect x="72" y="26" width="8" height="190" rx="4" fill="#fff" opacity=".22" filter="url(#d2-sf1)"/>
</g>`;
