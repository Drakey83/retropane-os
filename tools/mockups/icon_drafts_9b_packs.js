// Packs, redrawn (the owner, 2026-10-30: "i like the concept but i dont like the puzzle pieces").
// The pieces come from a generator, not hand-drawn outlines: each side of a square is
// flat (0), a knob (1) or a socket (-1), with a neck and a round head of fixed
// proportions, so any knob fits any socket of the same size.

const jigsaw = (S, [t, r, b, l]) => {
  // one side from p0 to p1; outward normal n; the head is a circle sampled as points
  const side = (p0, p1, n, kind) => {
    const at = (s, h) => [p0[0] + (p1[0] - p0[0]) * s + n[0] * h * kind * S, p0[1] + (p1[1] - p0[1]) * s + n[1] * h * kind * S];
    if (!kind) return [p1];
    const pts = [at(.36, 0), at(.4, .03), at(.41, .09)];
    // the head: a circle (centre .5, .21; radius .14, in edge units) from the left neck
    // point round the far side to the right neck point
    const c = [.5, .21], R = .14, aL = Math.atan2(.09 - c[1], .41 - c[0]), aR = Math.atan2(.09 - c[1], .59 - c[0]);
    const span = 2 * Math.PI - (aR - aL);
    for (let i = 0; i <= 28; i++) { const a = aL - i / 28 * span; pts.push(at(c[0] + R * Math.cos(a), c[1] + R * Math.sin(a))); }
    pts.push(at(.59, .09), at(.6, .03), at(.64, 0), p1);
    return pts;
  };
  const pts = [[0, 0], ...side([0, 0], [S, 0], [0, -1], t), ...side([S, 0], [S, S], [1, 0], r), ...side([S, S], [0, S], [0, 1], b), ...side([0, S], [0, 0], [-1, 0], l)];
  return 'M' + pts.map(p => p.map(v => v.toFixed(1)).join(' ')).join(' L') + ' Z';
};
// A glass piece: thickness below-right, glossy face, gloss laid over the face's own shape, and a
// white inner rim: the outline stroked and clipped to the piece, so it follows every knob and socket.
const piece = (p, x, y, S, sides, ramp, rot = 0) => {
  const d = jigsaw(S, sides);
  return `<defs>${lin(p + '-f', [[0, ramp[0]], [.5, ramp[1]], [1, ramp[2]]], 1, 1)}${lin(p + '-g', [[0, '#fff', .6], [.45, '#fff', .12], [.5, '#fff', 0]])}<clipPath id="${p}-c"><path d="${d}"/></clipPath></defs>
<g transform="translate(${x} ${y}) rotate(${rot} ${S / 2} ${S / 2})">
<path d="${d}" transform="translate(5 7)" fill="${ramp[2]}"/>
<path d="${d}" fill="url(#${p}-f)" stroke="${ramp[2]}" stroke-width="3" stroke-linejoin="round"/>
<path d="${d}" fill="url(#${p}-g)"/>
<path d="${d}" fill="none" stroke="#fff" stroke-width="9" opacity=".45" clip-path="url(#${p}-c)"/>
<path d="${d}" fill="none" stroke="${ramp[2]}" stroke-width="3" stroke-linejoin="round"/></g>`;
};

ICONS['Packs: two pieces'] = `
${shadow(128, 226, 104, 12, 'pk2-sh')}
${piece('pk2-a', 26, 64, 104, [0, 1, 0, 0], C.teal, -6)}
${piece('pk2-b', 150, 82, 84, [1, 0, 0, -1], C.orange, 8)}`;

ICONS['Packs: piece and plus'] = `
${shadow(124, 228, 96, 12, 'pk3-sh')}
${piece('pk3-a', 40, 56, 130, [1, 1, 0, 0], C.teal, 0)}
${badge('plus', 'pk3-p', 184, 182, 46)}`;

ICONS['Packs: set'] = `
${shadow(128, 232, 108, 12, 'pk4-sh')}
${piece('pk4-a', 28, 120, 92, [-1, 1, 0, 0], C.teal)}
${piece('pk4-b', 120, 120, 92, [1, 0, 0, -1], C.blue)}
${piece('pk4-c', 28, 28, 92, [0, 0, 1, 0], C.teal)}
${piece('pk4-d', 136, 22, 92, [0, 0, -1, 1], C.orange, 14)}`;

// The owner's pick (2026-10-30): "Pack set".
ICONS.Packs = ICONS['Packs: set'];
