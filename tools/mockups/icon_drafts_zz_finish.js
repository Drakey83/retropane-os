// The last pass over every icon (glass recipe 2, 2026-10-30): the shared filter set goes
// into each icon, and a soft drop shadow goes under it. Left out: the status-bar icons
// (blur muddies them at 16-24 px; they get the filters but no shadow) and Juanje's
// cards (his art, as he made it).
{
  const STATUS = new Set(['Battery full', 'Battery half', 'Battery low', 'Battery empty', 'Battery charging',
    'Wi-Fi 3', 'Wi-Fi 2', 'Wi-Fi 1', 'Wi-Fi off', 'Bluetooth on', 'Volume high', 'Volume low', 'Volume mute',
    'Notification', 'Alarm set', 'Sync running', 'Card inserted', 'Locked']);
  for (const n of Object.keys(ICONS)) {
    if (n.includes('(Juanje)')) continue;
    ICONS[n] = FX_DEFS + (STATUS.has(n) ? ICONS[n] : `<g filter="url(#fx-drop)">${ICONS[n]}</g>`);
  }
}
