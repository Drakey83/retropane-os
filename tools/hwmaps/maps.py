# Draws docs/hw/img/gpio-map.svg and cpld-map.svg from the tables below.
# Keep these tables in step with docs/hw/datasheet.md sections 3 and 4
# (docs/CONTRIBUTOR-BRIEF.md: the datasheet follows the device).
# Run: python -I tools/hwmaps/maps.py docs/hw/img
import sys, os
out = sys.argv[1]

CAT = {
    'audio':   ('#f6c8d8', 'Audio (I2S, I2C, codec)'),
    'touch':   ('#cfe8c8', 'Touch / keys'),
    'store':   ('#fde3b8', 'SD / CF / Wi-Fi / Bluetooth'),
    'disp':    ('#c9dcf5', 'Display (2700G, backlight)'),
    'power':   ('#e3d3f2', 'Power, reset, voltage, USB'),
    'unknown': ('#e6e6e6', 'Job unknown (interrupt / wake)'),
    'none':    ('#ffffff', 'No known job on this board'),
}
G = {
    0: ('power', 'PWR btn'), 1: ('unknown', 'wake ?'), 3: ('power', 'VID'), 4: ('power', 'VID'),
    10: ('store', 'BT 32kHz'), 11: ('unknown', 'irq ?'), 12: ('store', 'SD det'), 13: ('store', 'CF det'), 14: ('power', 'USB cbl'),
    15: ('power', 'reboot'), 17: ('disp', 'PWM0 BL'), 18: ('disp', '2700G RDY'),
    23: ('touch', 'SSP1 CLK'), 24: ('touch', 'touch CS'), 25: ('touch', 'SSP1 TX'), 26: ('touch', 'SSP1 RX'),
    27: ('store', 'SD WP'), 28: ('audio', 'I2S BCLK'), 29: ('audio', 'I2S DIN'), 30: ('audio', 'I2S DOUT'),
    31: ('audio', 'I2S SYNC'), 32: ('store', 'MMCLK'), 36: ('unknown', 'chg ?'), 38: ('power', 'VID'),
    42: ('store', 'BT RXD'), 43: ('store', 'BT TXD'), 44: ('store', 'BT CTS'), 45: ('store', 'BT RTS'),
    82: ('audio', 'codec gate'), 83: ('audio', 'HP jack'), 84: ('store', 'CF ready'), 85: ('disp', '2700G irq'),
    86: ('unknown', 'irq ?'), 87: ('unknown', 'irq ?'), 89: ('store', 'WiFi rst'), 94: ('touch', 'pen'),
    96: ('audio', 'audio on'), 99: ('power', 'board rst'), 107: ('store', 'WiFi irq'), 113: ('audio', 'I2S SYSCLK'),
    117: ('audio', 'I2C SCL'), 118: ('audio', 'I2C SDA'),
}
cols, cw, ch, x0, y0 = 11, 82, 46, 20, 60
rows = (121 + cols - 1) // cols
W = x0 * 2 + cols * cw
H = y0 + rows * ch + 120
s = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" font-family="Segoe UI, Helvetica, Arial, sans-serif">',
     '<title>Dell Axim X51v: PXA270 GPIO map</title>',
     f'<rect width="{W}" height="{H}" fill="#fff"/>',
     f'<text x="{x0}" y="32" font-size="17" font-weight="600">PXA270 GPIOs 0-120 on the Dell Axim X51v (RetroPane, 2026-10-18)</text>',
     f'<text x="{x0}" y="50" font-size="11" fill="#444">Lines with a known or suspected job; confidence and evidence per line in docs/hw/datasheet.md section 3. The keypad uses its controller pins (keypad.md).</text>']
for n in range(121):
    r, c = divmod(n, cols)
    x, y = x0 + c * cw, y0 + r * ch
    cat, lab = G.get(n, ('none', ''))
    fill = CAT[cat][0]
    s.append(f'<rect x="{x+2}" y="{y+2}" width="{cw-4}" height="{ch-4}" rx="4" fill="{fill}" stroke="#888" stroke-width="0.8"/>')
    s.append(f'<text x="{x+8}" y="{y+18}" font-size="12" font-weight="600">{n}</text>')
    if lab:
        s.append(f'<text x="{x+8}" y="{y+34}" font-size="10.5" fill="#222">{lab}</text>')
ly = y0 + rows * ch + 24
for i, (k, (fill, name)) in enumerate(CAT.items()):
    lx = x0 + (i % 4) * 225
    yy = ly + (i // 4) * 26
    s.append(f'<rect x="{lx}" y="{yy-12}" width="16" height="16" rx="3" fill="{fill}" stroke="#888" stroke-width="0.8"/>')
    s.append(f'<text x="{lx+22}" y="{yy+1}" font-size="12">{name}</text>')
s.append(f'<text x="{x0}" y="{ly+66}" font-size="11" fill="#444">VID = core-voltage select (GPIO3/4/38). GPIO83 high = headphones in (codec powered). GPIO12/13 low = card in. GPIO99 high = board reset.</text>')
s.append('</svg>')
open(os.path.join(out, 'gpio-map.svg'), 'w', encoding='utf-8', newline='\n').write('\n'.join(s) + '\n')

# CPLD: (offset, bit) -> (confidence, label)
C = {
    (0x02, 6): ('likely', 'Wi-Fi seq 1'),
    (0x04, 1): ('likely', 'CF present (0 = in) [read]'),
    (0x08, 1): ('likely', 'reboot (kernel)'),
    (0x10, 2): ('likely', 'BT power 2'), (0x10, 3): ('likely', 'Wi-Fi seq 5'),
    (0x10, 12): ('confirmed', 'CF power 2'), (0x10, 13): ('likely', 'Wi-Fi seq 4'),
    (0x10, 15): ('guess', 'notify'),
    (0x12, 0): ('guess', 'disp seq'), (0x12, 1): ('guess', 'disp seq'), (0x12, 2): ('guess', 'disp seq'), (0x12, 3): ('guess', 'disp seq'),
    (0x14, 1): ('confirmed', 'speaker amp'), (0x14, 2): ('guess', 'periph'), (0x14, 3): ('danger', 'display: DO NOT CLEAR'),
    (0x16, 0): ('confirmed', 'SD power'), (0x16, 1): ('confirmed', 'CF power 1'), (0x16, 2): ('likely', 'Wi-Fi seq 3'), (0x16, 3): ('likely', 'BT power 1'),
    (0x18, 0): ('likely', 'BT reset (low)'), (0x18, 1): ('likely', 'CF reset'), (0x18, 2): ('confirmed', 'backlight on'), (0x18, 3): ('guess', 'display'),
    (0x1A, 1): ('danger', 'battery?: DO NOT CLEAR'), (0x1A, 3): ('guess', 'notify'),
    (0x1C, 0): ('likely', 'Wi-Fi seq 2'), (0x1C, 1): ('confirmed', 'codec power'), (0x1C, 3): ('likely', 'USB / dock'),
    (0x1E, 0): ('confirmed', 'LCD type [read] batt? [write]'), (0x1E, 1): ('confirmed', 'LCD type [read] batt? [write]'), (0x1E, 2): ('guess', 'battery'),
    (0x1E, 3): ('likely', 'LCD read enable'),
}
CC = {'confirmed': '#9fd39a', 'likely': '#cfe6f7', 'guess': '#f7efc0', 'danger': '#f4a6a6', 'none': '#ffffff'}
bw, rh, lx0, ty = 54, 30, 70, 84
W = lx0 + 16 * bw + 20
H = ty + 16 * rh + 110
s = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" font-family="Segoe UI, Helvetica, Arial, sans-serif">',
     '<title>Dell Axim X51v: board CPLD bit map</title>',
     f'<rect width="{W}" height="{H}" fill="#fff"/>',
     '<text x="20" y="30" font-size="17" font-weight="600">Board CPLD at 0x10000000: 16 x 16-bit write-only latches (RetroPane, 2026-10-18)</text>',
     '<text x="20" y="48" font-size="11" fill="#444">Writes do not read back: always change bits through the shared shadow table (Cpld::adopt). A test image never switches a bit with any recorded owner.</text>']
for b in range(16):
    x = lx0 + (15 - b) * bw
    s.append(f'<text x="{x + bw/2}" y="{ty-8}" font-size="11" text-anchor="middle" fill="#444">bit {b}</text>')
for i in range(16):
    off = i * 2
    y = ty + i * rh
    s.append(f'<text x="20" y="{y+19}" font-size="12" font-family="Consolas, monospace">0x{off:02X}</text>')
    for b in range(16):
        x = lx0 + (15 - b) * bw
        conf, lab = C.get((off, b), ('none', ''))
        s.append(f'<rect x="{x+1}" y="{y+1}" width="{bw-2}" height="{rh-2}" rx="3" fill="{CC[conf]}" stroke="#aaa" stroke-width="0.7"/>')
        if lab:
            words = lab.split(' ')
            if len(lab) > 9 and len(words) > 1:
                h = (len(words) + 1) // 2
                l1, l2 = ' '.join(words[:h]), ' '.join(words[h:])
                s.append(f'<text x="{x+bw/2}" y="{y+13}" font-size="8.5" text-anchor="middle">{l1}</text>')
                s.append(f'<text x="{x+bw/2}" y="{y+24}" font-size="8.5" text-anchor="middle">{l2}</text>')
            else:
                s.append(f'<text x="{x+bw/2}" y="{y+19}" font-size="9" text-anchor="middle">{lab}</text>')
ly = ty + 16 * rh + 28
for i, (k, name) in enumerate([('confirmed', 'confirmed (measured)'), ('likely', 'likely (Dell code / AxDroid)'),
                               ('guess', 'guess'), ('danger', 'clearing it wedged the device'), ('none', 'no known user')]):
    x = 20 + i * 185 + (30 if i == 4 else 0)
    s.append(f'<rect x="{x}" y="{ly-12}" width="16" height="16" rx="3" fill="{CC[k]}" stroke="#888" stroke-width="0.8"/>')
    s.append(f'<text x="{x+22}" y="{ly+1}" font-size="12">{name}</text>')
s.append(f'<text x="20" y="{ly+30}" font-size="11" fill="#444">[read] = the offset reads a status input instead of the latch. LCD type: 0 = 3.7" Sharp VGA (the owner\'s), 1/3 = 3.5" Sharp QVGA, 2 = 3.7" Samsung VGA.</text>')
s.append(f'<text x="20" y="{ly+48}" font-size="11" fill="#444">Owners, sequences and evidence per bit: docs/hw/cpld.md.</text>')
s.append('</svg>')
open(os.path.join(out, 'cpld-map.svg'), 'w', encoding='utf-8', newline='\n').write('\n'.join(s) + '\n')
print('ok')
