// Icon drafts, batch 6: the status bar and the shade (icon-brief.md "The full set").
// These show at 16-24 px, so each is one bold shape with the glass kept to a gloss.

const battery = (p, level, charging = false) => {
  const ramp = level <= .15 ? C.red : level <= .35 ? C.orange : C.green, w = Math.max(10, 176 * level);
  return `${shadow(128, 206, 108, 11, p + '-sh')}
<defs>${lin(p + '-gl', [[0, '#ffffff', .9], [.5, '#d8e2ec', .55], [1, '#8592a2', .9]])}${lin(p + '-fl', [[0, ramp[0]], [.5, ramp[1]], [1, ramp[2]]])}${lin(p + '-cp', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']])}${lin(p + '-bl', [[0, '#fff6c0'], [1, '#f2a01c']])}</defs>
<rect x="212" y="102" width="28" height="52" rx="6" fill="url(#${p}-cp)" stroke="#2a3038" stroke-width="3"/>
<rect x="14" y="72" width="204" height="112" rx="20" fill="url(#${p}-gl)" stroke="#2a3038" stroke-width="6"/>
<rect x="28" y="86" width="${w.toFixed(0)}" height="84" rx="10" fill="url(#${p}-fl)" stroke="${ramp[2]}" stroke-width="2"/>
<rect x="24" y="80" width="186" height="34" rx="12" fill="#fff" opacity=".55"/>
${charging ? `<path d="M138 50 L84 136 H120 L104 206 L170 112 H132 Z" fill="url(#${p}-bl)" stroke="#5a3a04" stroke-width="6" stroke-linejoin="round"/>` : ''}`;
};
ICONS['Battery full'] = battery('ba1', 1);
ICONS['Battery half'] = battery('ba2', .5);
ICONS['Battery low'] = battery('ba3', .25);
ICONS['Battery empty'] = battery('ba4', .08);
ICONS['Battery charging'] = battery('ba5', .6, true);

const wifi = (p, level) => `${shadow(128, 226, 70, 10, p + '-sh')}
<defs>${lin(p + '-a', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
${[134, 92, 50].map((r, i) => { const on = 3 - i <= level, P = a => `${(128 + r * Math.cos(a * Math.PI / 180)).toFixed(1)} ${(198 + r * Math.sin(a * Math.PI / 180)).toFixed(1)}`, d = `M${P(222)} A${r} ${r} 0 0 1 ${P(318)}`; return on ? `<path d="${d}" fill="none" stroke="#0a2a60" stroke-width="32" stroke-linecap="round"/><path d="${d}" fill="none" stroke="url(#${p}-a)" stroke-width="26" stroke-linecap="round"/><path d="M${P(234)} A${r + 5} ${r + 5} 0 0 1 ${P(290)}" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" opacity=".7"/>` : `<path d="${d}" fill="none" stroke="#5c6672" stroke-width="32" stroke-linecap="round" opacity=".35"/><path d="${d}" fill="none" stroke="#c4ccd6" stroke-width="26" stroke-linecap="round" opacity=".35"/>`; }).join('')}
${level > 0 ? orb(p + '-dot', 128, 198, 22, C.blue, .7) : `<circle cx="128" cy="198" r="22" fill="#c4ccd6" opacity=".5" stroke="#5c6672" stroke-width="3"/>`}`;
ICONS['Wi-Fi 3'] = wifi('wf3', 3);
ICONS['Wi-Fi 2'] = wifi('wf2', 2);
ICONS['Wi-Fi 1'] = wifi('wf1', 1);
ICONS['Wi-Fi off'] = wifi('wf0', 0) + `<path d="M40 40 L216 216" stroke="#fff" stroke-width="34" stroke-linecap="round"/><path d="M40 40 L216 216" stroke="#d0281a" stroke-width="22" stroke-linecap="round"/>`;

const speaker = (p, waves, mute = false) => `${shadow(128, 226, 90, 11, p + '-sh')}
<defs>${lin(p + '-bx', [[0, '#9aa6b6'], [.3, '#3c4450'], [1, '#14181e']], 1, 0)}${lin(p + '-w', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
<path d="M22 92 H66 L130 36 V216 L66 160 H22 Z" fill="none" stroke="#fff" stroke-width="22" stroke-linejoin="round" opacity=".85"/>
<path d="M22 92 H66 L130 36 V216 L66 160 H22 Z" fill="url(#${p}-bx)" stroke="#05060a" stroke-width="6" stroke-linejoin="round"/>
<path d="M28 98 H66 L124 48 V98 Q74 124 28 118 Z" fill="#fff" opacity=".35"/>
${[48, 88].slice(0, waves).map(r => { const P = a => `${(130 + r * Math.cos(a * Math.PI / 180)).toFixed(1)} ${(126 + r * Math.sin(a * Math.PI / 180)).toFixed(1)}`; return `<path d="M${P(-50)} A${r} ${r} 0 0 1 ${P(50)}" fill="none" stroke="#0a2a60" stroke-width="26" stroke-linecap="round"/><path d="M${P(-50)} A${r} ${r} 0 0 1 ${P(50)}" fill="none" stroke="url(#${p}-w)" stroke-width="18" stroke-linecap="round"/>`; }).join('')}
${mute ? `<path d="M160 90 L226 156 M226 90 L160 156" stroke="#fff" stroke-width="34" stroke-linecap="round"/><path d="M160 90 L226 156 M226 90 L160 156" stroke="#d0281a" stroke-width="22" stroke-linecap="round"/>` : ''}`;
ICONS['Volume high'] = speaker('vh', 2);
ICONS['Volume low'] = speaker('vl', 1);
ICONS['Volume mute'] = speaker('vm', 0, true);

ICONS['Bluetooth on'] = ICONS.Bluetooth;

ICONS.Notification = `
${shadow(128, 228, 84, 12, 'nf-sh')}
<defs>${lin('nf-b', [[0, '#fff6c0'], [.45, '#f2c01c'], [1, '#9a6a04']], 1, 1)}</defs>
<path d="M128 26 Q186 26 190 98 Q192 156 224 184 Q230 194 218 196 H38 Q26 194 32 184 Q64 156 66 98 Q70 26 128 26 Z" fill="url(#nf-b)" stroke="#5a3a04" stroke-width="6" stroke-linejoin="round"/>
<path d="M80 98 Q82 40 128 38 Q160 38 172 64 Q128 60 80 98 Z" fill="#fff" opacity=".55"/>
<path d="M100 200 A28 28 0 0 0 156 200 Z" fill="url(#nf-b)" stroke="#5a3a04" stroke-width="5"/>
<circle cx="194" cy="56" r="34" fill="#fff"/>${orb('nf-d', 194, 56, 28, C.red, .55)}`;

ICONS['Alarm set'] = `
${shadow(128, 232, 84, 12, 'al-sh')}
<defs>${rad('al-fc', [[0, '#ffffff'], [.85, '#eef3f8'], [1, '#c2ccd8']], .45, .4, .6)}${lin('al-bd', [[0, '#9aa6b6'], [.3, '#3c4450'], [1, '#14181e']], 1, 1)}</defs>
${orb('al-b1', 58, 58, 34, C.silver)}${orb('al-b2', 198, 58, 34, C.silver)}
<path d="M80 192 L58 230 M176 192 L198 230" stroke="#2a3038" stroke-width="16" stroke-linecap="round"/>
<circle cx="128" cy="132" r="92" fill="url(#al-bd)" stroke="#05060a" stroke-width="5"/>
<circle cx="128" cy="132" r="72" fill="url(#al-fc)"/>
<path d="M128 132 V82 M128 132 L166 154" stroke="#1c2026" stroke-width="14" stroke-linecap="round"/>
<circle cx="128" cy="132" r="9" fill="#e3321e"/>
<ellipse cx="110" cy="90" rx="44" ry="20" fill="#fff" opacity=".5" transform="rotate(-18 110 90)"/>`;

ICONS['Sync running'] = ICONS['Badge: Sync'];

ICONS['Card inserted'] = `
${shadow(128, 234, 74, 11, 'ci-sh')}
<defs>${lin('ci-c', [[0, '#c9f4ee'], [.45, '#3fb3a4'], [1, '#0e4a42']], 1, 1)}</defs>
<path d="M60 20 H164 L204 60 V222 Q204 232 194 232 H62 Q52 232 52 222 V28 Q52 20 60 20 Z" fill="url(#ci-c)" stroke="#0a3a34" stroke-width="6" stroke-linejoin="round"/>
<g fill="#fff3b0" stroke="#8a5a04" stroke-width="2">${[0, 1, 2, 3, 4].map(i => `<rect x="${70 + i * 22}" y="32" width="14" height="30" rx="3"/>`).join('')}</g>
<rect x="70" y="120" width="116" height="90" rx="6" fill="#fbfdff" stroke="#0a3a34" stroke-width="3"/>
${lines(82, 140, 92, 3, 20)}
<path d="M58 26 H160 L196 62 V96 Q128 116 58 100 Z" fill="#fff" opacity=".4"/>`;

ICONS['Locked'] = ICONS['Badge: Lock'];
