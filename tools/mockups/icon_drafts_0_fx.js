// Calendar and My Device, redrawn with glass recipe 2 (the owner, 2026-10-30: tried on
// three icons, then "Roll it out"); they replace the first drafts in icon_drafts.js.
// The recipe itself (glassFx, dropFx, softFx, FX_DEFS) lives in icon_drafts.js.

ICONS.Calendar = `
<defs>
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
<g>
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

ICONS['My Device'] = `
<defs>
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
<g>
<rect x="66" y="18" width="124" height="208" rx="18" fill="url(#d2-bd)" stroke="#0a0c10" stroke-width="3" filter="url(#d2-bdf)"/>
<g filter="url(#d2-scf)"><rect x="80" y="38" width="96" height="124" rx="3" fill="url(#d2-sc)"/>
<path d="M80 128 Q128 100 176 118 L176 162 L80 162 Z" fill="url(#d2-hl)"/>
<circle cx="150" cy="64" r="12" fill="#ffe27a"/></g>
<path d="M80 38 L176 38 L176 70 L80 118 Z" fill="url(#d2-rf)" filter="url(#d2-sf1)"/>
<circle cx="128" cy="194" r="17" fill="url(#d2-bt)" stroke="#2a3038" stroke-width="2" filter="url(#d2-btf)"/>
${[92, 164].map(x => `<rect x="${x - 10}" y="186" width="20" height="14" rx="4" fill="#5c6672" filter="url(#d2-btf)"/>`).join('')}
<rect x="72" y="26" width="8" height="190" rx="4" fill="#fff" opacity=".22" filter="url(#d2-sf1)"/>
</g>`;
