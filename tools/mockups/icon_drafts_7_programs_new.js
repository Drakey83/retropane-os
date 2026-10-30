// Programs, new ideas (the owner, 2026-10-30: "come up with a new one").
ICONS['Programs: cube'] = `
${shadow(128, 230, 96, 13, 'pc-sh')}
<defs>
${lin('pc-t', [[0, '#e4fbf8', .95], [1, '#3fb3a4', .9]], 1, 1)}
${lin('pc-l', [[0, '#8fd0ff', .92], [1, '#0d3f86', .95]])}
${lin('pc-r', [[0, '#ffd0a0', .92], [1, '#a03c02', .95]])}
</defs>
<path d="M128 20 L220 72 L128 124 L36 72 Z" fill="url(#pc-t)" stroke="#0e4a42" stroke-width="3" stroke-linejoin="round"/>
<path d="M36 72 L128 124 L128 232 L36 180 Z" fill="url(#pc-l)" stroke="#0a2a60" stroke-width="3" stroke-linejoin="round"/>
<path d="M128 124 L220 72 L220 180 L128 232 Z" fill="url(#pc-r)" stroke="#7a2c02" stroke-width="3" stroke-linejoin="round"/>
<g transform="matrix(1 .565 1 -.565 36 72)"><path d="${starPath(46, 46, 30)}" fill="#fff" opacity=".95"/></g>
<g transform="matrix(1 .565 0 1 36 72)"><path d="M33 30 H53 V44 H67 V64 H53 V78 H33 V64 H19 V44 H33 Z" fill="#fff" stroke="#fff" stroke-width="4" stroke-linejoin="round" opacity=".95"/></g>
<g transform="matrix(1 -.565 0 1 128 124)"><path d="M30 30 L70 22 L70 66 A11 8 0 1 1 62 58 L62 36 L38 41 L38 74 A11 8 0 1 1 30 66 Z" fill="#fff" opacity=".95"/></g>
${rim('M128 28 L210 74 L128 118 L46 74 Z', 2.5, .7)}
<path d="M40 78 L124 126 L124 150 Q80 134 40 108 Z" fill="#fff" opacity=".35"/>
<path d="M132 126 L216 78 L216 104 Q176 130 132 150 Z" fill="#fff" opacity=".35"/>
<path d="M128 124 L128 232" stroke="#fff" stroke-width="3" opacity=".6"/>`;

ICONS['Programs: rocket'] = `
${shadow(128, 232, 80, 11, 'pk-sh')}
<defs>
${lin('pk-bd', [[0, '#8592a2'], [.25, '#ffffff'], [.6, '#d4dbe3'], [1, '#5c6672']], 1, 0)}
${lin('pk-rd', [[0, '#ff9c8a'], [.5, '#e3321e'], [1, '#8a1004']], 1, 0)}
${lin('pk-fl', [[0, '#fff6c0'], [.4, '#ffb020'], [1, '#e3321e', 0]])}
</defs>
<g transform="rotate(38 128 128) translate(0 -6)">
<path d="M104 186 Q128 262 152 186 Z" fill="url(#pk-fl)"/>
<path d="M116 186 Q128 228 140 186 Z" fill="#fffbe0" opacity=".9"/>
<path d="M92 132 L56 186 Q54 194 62 192 L96 180 Z" fill="url(#pk-rd)" stroke="#6a0a02" stroke-width="3" stroke-linejoin="round"/>
<path d="M164 132 L200 186 Q202 194 194 192 L160 180 Z" fill="url(#pk-rd)" stroke="#6a0a02" stroke-width="3" stroke-linejoin="round"/>
<rect x="106" y="176" width="44" height="14" rx="4" fill="#4c5664" stroke="#1c2026" stroke-width="2"/>
<path d="M128 18 Q168 56 166 128 L164 180 L92 180 L90 128 Q88 56 128 18 Z" fill="url(#pk-bd)" stroke="#2a3038" stroke-width="3" stroke-linejoin="round"/>
<path d="M128 18 Q152 38 160 70 L96 70 Q104 38 128 18 Z" fill="url(#pk-rd)" stroke="#6a0a02" stroke-width="3" stroke-linejoin="round"/>
<circle cx="128" cy="110" r="24" fill="#d4dbe3" stroke="#2a3038" stroke-width="3"/>
${orb('pk-w', 128, 110, 17, C.blue, .7)}
<path d="M100 76 Q98 140 102 172" stroke="#fff" stroke-width="7" stroke-linecap="round" opacity=".7"/>
<path d="M120 170 L128 154 L136 170 Z" fill="#a9b4c0" opacity=".7"/></g>`;

ICONS['Programs: toolbox'] = `
${shadow(128, 228, 112, 13, 'pt-sh')}
<defs>
${lin('pt-bx', [[0, '#d6f8f3', .78], [.45, '#6fd0c2', .62], [1, '#1d8a7c', .9]])}
${lin('pt-ld', [[0, '#a8ece2'], [1, '#18786c']])}
${lin('pt-hd', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']])}
${lin('pt-sd', [[0, '#ffe0b0'], [.5, '#f58020'], [1, '#a03c02']], 1, 0)}
${lin('pt-mt', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']], 1, 0)}
</defs>
<path d="M88 72 V54 Q88 40 102 40 H154 Q168 40 168 54 V72" fill="none" stroke="#3a4250" stroke-width="16" stroke-linecap="round"/>
<path d="M88 72 V54 Q88 40 102 40 H154 Q168 40 168 54 V72" fill="none" stroke="url(#pt-hd)" stroke-width="10" stroke-linecap="round"/>
<g transform="rotate(-30 80 90)"><rect x="72" y="40" width="16" height="64" rx="7" fill="url(#pt-sd)" stroke="#7a2c02" stroke-width="2"/><rect x="77" y="100" width="6" height="40" fill="url(#pt-mt)"/></g>
<g transform="rotate(24 184 90)"><path d="M178 50 Q162 50 164 66 L176 72 V130 H192 V72 L204 66 Q206 50 190 50 L190 62 H178 Z" fill="url(#pt-mt)" stroke="#3a4250" stroke-width="2.5" stroke-linejoin="round"/></g>
${orb('pt-a', 128, 92, 20, C.purple, .6)}
<path d="M24 104 Q24 94 34 94 H222 Q232 94 232 104 V214 Q232 222 224 222 H32 Q24 222 24 214 Z" fill="url(#pt-bx)" stroke="#11574e" stroke-width="3"/>
<rect x="20" y="104" width="216" height="30" rx="6" fill="url(#pt-ld)" stroke="#11574e" stroke-width="3"/>
<rect x="112" y="124" width="32" height="24" rx="5" fill="#f2c01c" stroke="#8a5a04" stroke-width="2.5"/>
${rim('M30 140 H226 V214 H30 Z', 2.5, .7)}
${shine('M23 107 H233 V120 Q128 128 23 120 Z', 'pt-gl', .9)}
${glow('M30 196 H226 V216 H30 Z', 'pt-gw', .5)}
${streak(206, 142, 66, 8)}`;
