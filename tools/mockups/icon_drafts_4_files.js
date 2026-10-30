// Icon drafts, batch 4: the badges (overlays) and the file types (icon-brief.md "The full set").
// A badge is drawn at radius 100 around 0,0 and placed with badge(kind, p, cx, cy, r),
// so the same drawing serves a 256 px sheet entry and a corner overlay.

const BADGE = {
  shortcut: p => `<defs>${lin(p + 'b', [[0, '#ffffff'], [1, '#d8e2ec']])}${lin(p + 'a', [[0, '#c6ecff'], [.5, '#2a7fe0'], [1, '#0b3a80']])}</defs>
<rect x="-90" y="-90" width="180" height="180" rx="28" fill="url(#${p}b)" stroke="#5c6672" stroke-width="6"/>
<path d="M-50 60 Q-50 -26 30 -26 L30 -60 L80 -4 L30 52 L30 18 Q-20 18 -50 60 Z" fill="url(#${p}a)" stroke="#0a2a60" stroke-width="6" stroke-linejoin="round"/>
<path d="M-82 -82 H82 V-20 Q0 10 -82 -6 Z" fill="#fff" opacity=".45"/>`,
  check: p => `${orb(p + 'o', 0, 0, 92, C.green, .45)}<path d="M-48 2 L-14 38 L52 -38" fill="none" stroke="#fff" stroke-width="26" stroke-linecap="round" stroke-linejoin="round"/>`,
  plus: p => `${orb(p + 'o', 0, 0, 92, C.green, .45)}<path d="M0 -50 V50 M-50 0 H50" stroke="#fff" stroke-width="28" stroke-linecap="round"/>`,
  minus: p => `${orb(p + 'o', 0, 0, 92, C.red, .45)}<path d="M-50 0 H50" stroke="#fff" stroke-width="28" stroke-linecap="round"/>`,
  error: p => `${orb(p + 'o', 0, 0, 92, C.red, .45)}<path d="M-38 -38 L38 38 M38 -38 L-38 38" stroke="#fff" stroke-width="26" stroke-linecap="round"/>`,
  info: p => `${orb(p + 'o', 0, 0, 92, C.blue, .45)}<circle cx="0" cy="-46" r="15" fill="#fff"/><path d="M-20 -14 H8 V44 H22 M-22 44 H22" fill="none" stroke="#fff" stroke-width="22" stroke-linecap="round" stroke-linejoin="round"/>`,
  download: p => `${orb(p + 'o', 0, 0, 92, C.green, .45)}<path d="M-18 -60 H18 V-4 H46 L0 52 L-46 -4 H-18 Z" fill="#fff" stroke="#fff" stroke-width="6" stroke-linejoin="round"/>`,
  sync: p => `${orb(p + 'o', 0, 0, 92, C.blue, .35)}${arcArrow(0, 0, 50, 190, 330, '#fff', 20, 1.1)}${arcArrow(0, 0, 50, 10, 150, '#fff', 20, 1.1)}`,
  warning: p => `<defs>${lin(p + 't', [[0, '#fff6c0'], [.55, '#f2c01c'], [1, '#b07a04']])}</defs>
<path d="M0 -92 L96 76 Q100 86 88 86 L-88 86 Q-100 86 -96 76 Z" fill="url(#${p}t)" stroke="#6a4a04" stroke-width="7" stroke-linejoin="round"/>
<path d="M0 -70 L40 0 Q0 18 -40 0 Z" fill="#fff" opacity=".5"/>
<path d="M0 -40 V20" stroke="#1c2026" stroke-width="22" stroke-linecap="round"/><circle cx="0" cy="56" r="13" fill="#1c2026"/>`,
  lock: p => `<defs>${lin(p + 'g', [[0, '#fff3b0'], [.5, '#e8b020'], [1, '#8a5a04']])}${lin(p + 's', [[0, '#ffffff'], [.5, '#a9b4c0'], [1, '#4c5664']], 1, 0)}</defs>
<path d="M-50 -10 V-40 A50 50 0 0 1 50 -40 V-10" fill="none" stroke="#3a4250" stroke-width="30"/>
<path d="M-50 -10 V-40 A50 50 0 0 1 50 -40 V-10" fill="none" stroke="url(#${p}s)" stroke-width="22"/>
<rect x="-76" y="-14" width="152" height="108" rx="16" fill="url(#${p}g)" stroke="#6a4204" stroke-width="7"/>
<rect x="-68" y="-8" width="136" height="38" rx="12" fill="#fff" opacity=".45"/>
<circle cx="0" cy="34" r="14" fill="#5a3a04"/><rect x="-6" y="38" width="12" height="32" rx="5" fill="#5a3a04"/>`,
  star: p => `<defs>${rad(p + 's', [[0, '#fff6c0'], [.55, '#f2c01c'], [1, '#b07a04']], .4, .35, .7)}</defs>
<path d="${starPath(0, 6, 100)}" fill="url(#${p}s)" stroke="#8a5a04" stroke-width="6" stroke-linejoin="round"/>
<path d="${starPath(-6, -12, 44)}" fill="#fff" opacity=".45"/>`,
  magnifier: p => `<g transform="translate(-128 -128)">${lens(p + 'l', 108, 104, 66)}</g>`,
};
const badge = (kind, p, cx = 190, cy = 190, r = 52) => `<g transform="translate(${cx} ${cy}) scale(${r / 100})">${BADGE[kind](p)}</g>`;
for (const [name, kind] of [['Shortcut', 'shortcut'], ['Check', 'check'], ['Lock', 'lock'], ['Magnifier', 'magnifier'], ['Plus', 'plus'], ['Minus', 'minus'], ['Warning', 'warning'], ['Error', 'error'], ['Info', 'info'], ['Sync', 'sync'], ['Download', 'download'], ['Star', 'star']])
  ICONS['Badge: ' + name] = `${shadow(128, 232, 80, 12, 'bg-' + kind + '-sh')}${badge(kind, 'bg-' + kind, 128, 124, 100)}`;
// Badges on a base, as the OS lays them: lower left for a shortcut, lower right for state.
ICONS['Example: shortcut'] = ICONS.Notes + badge('shortcut', 'ex-s', 52, 196, 46);
ICONS['Example: locked'] = ICONS.Folder + badge('lock', 'ex-l', 196, 190, 50);
ICONS['Example: synced'] = ICONS['Documents folder'] + badge('check', 'ex-c', 196, 192, 46);

// File types: a page, what is in it, and (where it helps) a mark in the lower right.
const tile = (p, ramp, inner) => `<defs>${lin(p + '-t', [[0, ramp[0]], [.5, ramp[1]], [1, ramp[2]]])}</defs><rect x="136" y="150" width="84" height="84" rx="16" fill="url(#${p}-t)" stroke="${ramp[2]}" stroke-width="3"/><rect x="141" y="154" width="74" height="32" rx="12" fill="#fff" opacity=".45"/>${inner}`;
ICONS['Text file'] = page('ft', lines(68, 64, 116, 8, 18));
ICONS['Document file'] = page('fdc', `<rect x="68" y="58" width="70" height="11" rx="5" fill="#2a7fe0"/>${lines(68, 82, 116, 4, 18)}${tile('fdc', C.blue, `<text x="178" y="220" text-anchor="middle" font-family="Georgia, serif" font-weight="bold" font-size="66" fill="#fff">A</text>`)}`);
ICONS['Spreadsheet file'] = page('fss', `<g stroke="#b4bdc8" stroke-width="2">${[0, 1, 2, 3, 4, 5].map(i => `<path d="M68 ${60 + i * 20} H184"/>`).join('')}${[0, 1, 2, 3].map(i => `<path d="M${68 + i * 38.6} 60 V160"/>`).join('')}</g><rect x="68" y="60" width="116" height="20" fill="#36b02a" opacity=".85"/>${tile('fss', C.green, `<g fill="#fff"><rect x="152" y="196" width="14" height="26" rx="2"/><rect x="171" y="182" width="14" height="40" rx="2"/><rect x="190" y="168" width="14" height="54" rx="2"/></g>`)}`);
ICONS['Presentation file'] = page('fpr', `${lines(68, 60, 100, 2, 18)}<circle cx="104" cy="132" r="34" fill="#2a7fe0"/><path d="M110 126 L110 92 A34 34 0 0 1 143 134 Z" fill="#f58020" stroke="#a03c02" stroke-width="2"/>${tile('fpr', C.orange, `<rect x="152" y="172" width="52" height="36" rx="3" fill="#fff"/><path d="M178 208 V222 M166 224 H190" stroke="#fff" stroke-width="5" stroke-linecap="round"/>`)}`);
ICONS['PDF file'] = page('fpd', `${lines(68, 60, 116, 5, 18)}<defs>${lin('fpd-b', [[0, '#ff9c8a'], [.5, '#d0281a'], [1, '#7a0a04']])}</defs><rect x="40" y="160" width="176" height="56" rx="10" fill="url(#fpd-b)" stroke="#6a0a02" stroke-width="3"/><rect x="45" y="164" width="166" height="22" rx="8" fill="#fff" opacity=".4"/><text x="128" y="203" text-anchor="middle" font-family="Arial, sans-serif" font-weight="bold" font-size="40" fill="#fff">PDF</text>`);
ICONS['Picture file'] = page('fpc', `<defs>${lin('fpc-s', [[0, '#7cc6ff'], [1, '#d8f0ff']])}${lin('fpc-h', [[0, '#8fe07a'], [1, '#2a8a20']])}</defs><rect x="66" y="74" width="122" height="120" fill="url(#fpc-s)" stroke="#8592a2" stroke-width="2"/><circle cx="160" cy="102" r="13" fill="#ffe27a"/><path d="M66 172 Q96 126 128 156 Q156 130 188 160 L188 194 L66 194 Z" fill="url(#fpc-h)"/>`);
ICONS['Music file'] = page('fmu', `${lines(68, 60, 70, 3, 18)}<defs>${lin('fmu-n', [[0, '#a8dcff'], [.5, '#2a7fe0'], [1, '#0c3c88']])}</defs><path d="M110 112 L182 94 L182 186 A20 15 0 1 1 166 172 L166 124 L126 134 L126 204 A20 15 0 1 1 110 190 Z" fill="url(#fmu-n)" stroke="#0a2a60" stroke-width="3"/><path d="M116 117 L176 102 L176 112 L116 127 Z" fill="#fff" opacity=".7"/>`);
ICONS['Video file'] = page('fvi', `<defs>${lin('fvi-f', [[0, '#4a525e'], [1, '#14181e']])}</defs><rect x="62" y="70" width="130" height="130" rx="4" fill="url(#fvi-f)" stroke="#000" stroke-width="2"/>${[0, 1, 2, 3, 4, 5].map(i => `<rect x="67" y="${76 + i * 21}" width="10" height="12" rx="2" fill="#dfe5ec"/><rect x="177" y="${76 + i * 21}" width="10" height="12" rx="2" fill="#dfe5ec"/>`).join('')}<rect x="84" y="78" width="86" height="114" fill="#3a92ea"/><path d="M114 112 L148 135 L114 158 Z" fill="#fff"/><rect x="84" y="78" width="86" height="40" fill="#fff" opacity=".25"/>`);
ICONS['Archive (zip)'] = page('fzp', `<defs>${lin('fzp-z', [[0, '#e6ebf0'], [.5, '#8592a2'], [1, '#3c4450']], 1, 0)}${lin('fzp-b', [[0, '#c9f4ee'], [.5, '#3fb3a4'], [1, '#11574e']])}</defs>
<rect x="112" y="22" width="30" height="190" fill="#e8eef4"/>${[...Array(11)].map((_, i) => `<rect x="${i % 2 ? 127 : 113}" y="${26 + i * 12}" width="14" height="8" rx="2" fill="url(#fzp-z)" stroke="#3c4450" stroke-width="1"/>`).join('')}
<rect x="113" y="158" width="28" height="36" rx="5" fill="url(#fzp-z)" stroke="#3c4450" stroke-width="2"/><rect x="120" y="190" width="14" height="22" rx="5" fill="none" stroke="#3c4450" stroke-width="4"/>
<rect x="52" y="100" width="152" height="36" fill="url(#fzp-b)" stroke="#0e4a42" stroke-width="3"/><rect x="55" y="103" width="146" height="12" rx="5" fill="#fff" opacity=".5"/>`);
ICONS['Program file'] = `${shadow(128, 228, 104, 13, 'fex-sh')}${win('fex', 26, 36, 204, 176, C.blue)}${place(ICONS.Settings.replace(/st-/g, 'fexg-'), 128, 140, .6)}`;
ICONS['Install package (.cab)'] = `
${shadow(128, 230, 106, 14, 'fcb-sh')}
<defs>${lin('fcb-fr', [[0, '#d6f8f3', .8], [.45, '#6fd0c2', .65], [1, '#1d8a7c', .9]])}${lin('fcb-tp', [[0, '#e4fbf8'], [1, '#3fb3a4']])}${lin('fcb-sd', [[0, '#3fb3a4'], [1, '#11574e']])}</defs>
<path d="M40 104 L74 76 L238 76 L204 104 Z" fill="url(#fcb-tp)" stroke="#11574e" stroke-width="3" stroke-linejoin="round"/>
<path d="M204 104 L238 76 L238 190 L204 220 Z" fill="url(#fcb-sd)" stroke="#11574e" stroke-width="3" stroke-linejoin="round"/>
<rect x="40" y="104" width="164" height="116" fill="url(#fcb-fr)" stroke="#11574e" stroke-width="3"/>
${rim('M46 110 H198 V214 H46 Z', 2.5, .7)}
<path d="M110 104 L144 76 L162 76 L128 104 Z" fill="#f2c01c" opacity=".85"/><rect x="110" y="104" width="18" height="116" fill="#f2c01c" opacity=".8"/>
${shine('M43 107 H201 V140 Q122 152 43 140 Z', 'fcb-gl', .8)}
${badge('download', 'fcb-d', 184, 70, 44)}`;
ICONS['Font file'] = page('ffn', `<text x="122" y="168" text-anchor="middle" font-family="Georgia, serif" font-size="100" fill="#2b3644">Aa</text><path d="M68 190 H184" stroke="#b4bdc8" stroke-width="5"/>`);
ICONS['Web page'] = page('fwb', `${lines(68, 60, 80, 3, 18)}${place(ICONS.Browser.replace(/br-/g, 'fwbg-'), 128, 160, .54)}`);
ICONS['Contact card (vCard)'] = `
${shadow(128, 214, 108, 13, 'fvc-sh')}
<defs>${lin('fvc-c', [[0, '#ffffff'], [1, '#dce5ee']])}${rad('fvc-p', [[0, '#d8f0ff'], [.5, '#3a92ea'], [1, '#0b3a80']], .38, .3, .8)}</defs>
<rect x="14" y="58" width="228" height="148" rx="14" fill="url(#fvc-c)" stroke="#6d7886" stroke-width="3"/>
<rect x="14" y="58" width="228" height="30" rx="14" fill="#2a7fe0"/><rect x="14" y="76" width="228" height="12" fill="#2a7fe0"/>
<rect x="34" y="104" width="76" height="86" rx="8" fill="#e6eef6" stroke="#8592a2" stroke-width="2"/>
<circle cx="72" cy="132" r="18" fill="url(#fvc-p)"/><path d="M44 186 Q44 154 72 154 Q100 154 100 186 Z" fill="url(#fvc-p)"/>
${lines(124, 108, 98, 4, 20)}
${shine('M17 61 H239 V110 Q128 128 17 116 Z', 'fvc-gl', .5)}`;
ICONS['Calendar file'] = page('fcl', `<defs>${lin('fcl-h', [[0, '#ff9c80'], [.5, '#e8402a'], [1, '#a0180a']])}</defs><rect x="64" y="70" width="124" height="120" rx="6" fill="#fff" stroke="#8592a2" stroke-width="2"/><path d="M64 76 Q64 70 70 70 H182 Q188 70 188 76 V100 H64 Z" fill="url(#fcl-h)"/><g fill="#c6ced8">${[0, 1, 2, 3, 4].map(i => [0, 1, 2, 3].map(j => `<rect x="${74 + i * 22}" y="${110 + j * 18}" width="16" height="11" rx="2"/>`).join('')).join('')}</g><rect x="118" y="146" width="16" height="11" rx="2" fill="#e8402a"/>`);
ICONS['Note file'] = page('fnt', `<path d="M54 26 L164 24 L202 62 L202 226 L54 226 Z" fill="#ffeb7a" opacity=".85"/>${lines(68, 70, 112, 7, 20, '#d7a52a')}`);
ICONS['Unknown file'] = page('fuk', `<text x="128" y="176" text-anchor="middle" font-family="Georgia, serif" font-weight="bold" font-size="120" fill="#a9b4c0">?</text>`);
