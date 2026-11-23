//! Word suggestions for a note's text (notes-rule0.md, "Sentence capitals
//! and word suggestions").
//!
//! Iterate, don't imitate (rule 0): Windows Mobile 5's word completion
//! carried a fixed list in ROM and never learned a name; Android's
//! LatinIME carries a 1-2 MB trie a language and a learning model. On the
//! X51v (64 MB, the program's 256 KB slot) ours is a ranked list of the
//! words notes use most, **front-coded** at build time (`build.rs`: sorted,
//! sixteen a block, each word stored as the bytes it shares with the one
//! before plus its own, and a rank byte), read in place from the program's
//! constants: no RAM, a binary search over the blocks and a scan of one
//! prefix's range. It **learns** from the user's own notes at no storage
//! cost: the words already written (names, places, jargon) are counted from
//! the store when the keyboard opens and offered beside the list's, ranked
//! by how often the user wrote them. Any language is a word list dropped in
//! `words/` (US English the default, the owner's); capitals follow what
//! was typed (`Port` gives `Portland`, `NA` gives `NASA`).

include!(concat!(env!("OUT_DIR"), "/dicts.rs"));

/// The longest word offered.
pub const WORD_MAX: usize = 24;

/// A packed dictionary read in place.
#[derive(Clone, Copy, Debug)]
pub struct Dict<'a> {
    data: &'a [u8],
    blocks: usize,
    count: usize,
}

impl<'a> Dict<'a> {
    /// `data` as a dictionary, `None` where it is not one (checked once:
    /// every read after is bounds-checked anyway).
    #[must_use]
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.get(..5)? != b"RPWD1" {
            return None;
        }
        let count = usize::from(u16::from_le_bytes([*data.get(5)?, *data.get(6)?]));
        let blocks = usize::from(u16::from_le_bytes([*data.get(7)?, *data.get(8)?]));
        (9 + 2 * blocks <= data.len()).then_some(Self {
            data,
            blocks,
            count,
        })
    }

    /// The default (US English).
    #[must_use]
    pub fn default_lang() -> Option<Self> {
        DICTS
            .iter()
            .find(|d| d.0 == "en-US")
            .and_then(|d| Self::new(d.1))
    }

    /// How many words.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether it holds none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The bytes it takes.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.data.len()
    }

    fn block_start(&self, b: usize) -> Option<usize> {
        let at = 9 + 2 * b;
        let off = usize::from(u16::from_le_bytes([
            *self.data.get(at)?,
            *self.data.get(at + 1)?,
        ]));
        Some(9 + 2 * self.blocks + off)
    }

    /// Each word of block `b` given to `f` (the word, its rank) until `f`
    /// answers false; false when it stopped.
    fn each_in(&self, b: usize, f: &mut dyn FnMut(&[u8], u8) -> bool) -> bool {
        let Some(mut at) = self.block_start(b) else {
            return true;
        };
        let end = self
            .block_start(b + 1)
            .unwrap_or(self.data.len())
            .min(self.data.len());
        let mut word = [0u8; 32];
        while at + 2 <= end {
            let shared = usize::from(self.data[at]);
            let own = usize::from(self.data[at + 1]);
            let Some(bytes) = self.data.get(at + 2..at + 2 + own) else {
                return true;
            };
            let Some(&rank) = self.data.get(at + 2 + own) else {
                return true;
            };
            if shared + own > word.len() {
                return true;
            }
            word[shared..shared + own].copy_from_slice(bytes);
            if !f(&word[..shared + own], rank) {
                return false;
            }
            at += 3 + own;
        }
        true
    }

    /// Every word starting with `prefix` (lower case) given to `f` with its
    /// rank.
    pub fn starting(&self, prefix: &[u8], f: &mut dyn FnMut(&[u8], u8)) {
        // The last block whose first word is below the prefix.
        let (mut lo, mut hi) = (0usize, self.blocks);
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            let mut first = [0u8; 32];
            let mut fl = 0;
            self.each_in(mid, &mut |w, _| {
                fl = w.len();
                first[..fl].copy_from_slice(w);
                false
            });
            if &first[..fl] < prefix {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        for b in lo..self.blocks {
            let more = self.each_in(b, &mut |w, r| {
                if w.starts_with(prefix) {
                    f(w, r);
                    true
                } else {
                    w < prefix
                }
            });
            if !more {
                break;
            }
        }
    }

    /// Whether `word` (lower case) is in the list.
    #[must_use]
    pub fn has(&self, word: &[u8]) -> bool {
        let mut found = false;
        self.starting(word, &mut |w, _| found |= w == word);
        found
    }
}

/// Words the user wrote, counted (learned from the store).
pub struct Learned {
    words: [([u8; WORD_MAX], u8, u16); 96],
    n: usize,
}

impl Default for Learned {
    fn default() -> Self {
        Self::new()
    }
}

impl Learned {
    /// None yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            words: [([0; WORD_MAX], 0, 0); 96],
            n: 0,
        }
    }

    /// Forgets them all.
    pub fn clear(&mut self) {
        self.n = 0;
    }

    /// The words of `text` (letters and apostrophes, three or more, lower
    /// cased) counted, those of `dict` left to it. When full, a word seen
    /// once makes room for a new one.
    pub fn count_in(&mut self, text: &[u8], dict: Option<&Dict<'_>>) {
        for w in text.split(|c| !(c.is_ascii_alphabetic() || *c == b'\'')) {
            if w.len() < 3 || w.len() > WORD_MAX {
                continue;
            }
            let mut low = [0u8; WORD_MAX];
            for (o, c) in low.iter_mut().zip(w) {
                *o = c.to_ascii_lowercase();
            }
            let low = &low[..w.len()];
            if dict.is_some_and(|d| d.has(low)) {
                continue;
            }
            if let Some(e) = self.words[..self.n]
                .iter_mut()
                .find(|e| &e.0[..usize::from(e.1)] == low)
            {
                e.2 = e.2.saturating_add(1);
                continue;
            }
            let slot = if self.n < self.words.len() {
                self.n += 1;
                Some(self.n - 1)
            } else {
                self.words.iter().position(|e| e.2 <= 1)
            };
            if let Some(k) = slot {
                self.words[k].0[..low.len()].copy_from_slice(low);
                self.words[k].1 = low.len() as u8;
                self.words[k].2 = 1;
            }
        }
    }

    /// Each learned word starting with `prefix`, with how often.
    pub fn starting(&self, prefix: &[u8], f: &mut dyn FnMut(&[u8], u16)) {
        for e in &self.words[..self.n] {
            let w = &e.0[..usize::from(e.1)];
            if w.starts_with(prefix) {
                f(w, e.2);
            }
        }
    }
}

/// Up to three suggestions for the word being typed (`typed`, as the user
/// typed it): the learned words a user wrote twice or more first, then the
/// list's by rank; never the typed word itself; capitals as typed.
pub fn suggest(
    typed: &[u8],
    dict: Option<&Dict<'_>>,
    learned: &Learned,
    out: &mut [[u8; WORD_MAX]; 3],
) -> [usize; 3] {
    let mut lens = [0usize; 3];
    if typed.is_empty()
        || typed.len() >= WORD_MAX
        || !typed.iter().all(|c| c.is_ascii_alphabetic() || *c == b'\'')
    {
        return lens;
    }
    let mut low = [0u8; WORD_MAX];
    for (o, c) in low.iter_mut().zip(typed) {
        *o = c.to_ascii_lowercase();
    }
    let prefix = &low[..typed.len()];
    // Candidates scored: lower is better (learned words by count, then
    // the list's ranks after them).
    let mut best: [([u8; WORD_MAX], usize, u32); 3] = [([0; WORD_MAX], 0, u32::MAX); 3];
    let mut offer = |w: &[u8], score: u32| {
        if w.len() == prefix.len()
            || w.len() > WORD_MAX
            || best.iter().any(|b| b.2 != u32::MAX && &b.0[..b.1] == w)
        {
            return;
        }
        if let Some(k) = best.iter().position(|b| score < b.2) {
            best.copy_within(k..2, k + 1);
            best[k].0[..w.len()].copy_from_slice(w);
            best[k].1 = w.len();
            best[k].2 = score;
        }
    };
    learned.starting(prefix, &mut |w, n| {
        if n >= 2 {
            offer(w, 1000 - u32::from(n.min(999)));
        }
    });
    if let Some(d) = dict {
        d.starting(prefix, &mut |w, r| {
            offer(w, 1000 + u32::from(r) * 4 + w.len() as u32)
        });
    }
    learned.starting(prefix, &mut |w, n| {
        if n < 2 {
            offer(w, 3000);
        }
    });
    let upper_first = typed[0].is_ascii_uppercase();
    let all_upper = typed.len() >= 2 && typed.iter().all(|c| !c.is_ascii_lowercase());
    for (k, b) in best.iter().enumerate() {
        if b.2 == u32::MAX {
            continue;
        }
        let w = &b.0[..b.1];
        for (i, &c) in w.iter().enumerate() {
            out[k][i] = if all_upper || (upper_first && i == 0) {
                c.to_ascii_uppercase()
            } else {
                c
            };
        }
        lens[k] = w.len();
    }
    lens
}

/// Whether the next letter typed after `before` (the text before the
/// caret, in its line; `line_start` when the caret begins a block) starts
/// a sentence: at a block's start, or after `.`, `!` or `?` and a space.
#[must_use]
pub fn sentence_starts(before: &[u8]) -> bool {
    let t = before
        .strip_prefix(b"[ ] ")
        .or_else(|| before.strip_prefix(b"[x] "))
        .unwrap_or(before);
    let t = t.strip_prefix(&[crate::doc::CONT]).unwrap_or(t);
    let trimmed = t.trim_ascii_end();
    if trimmed.is_empty() {
        // A continuation goes on with its sentence.
        return before.first() != Some(&crate::doc::CONT);
    }
    trimmed.len() < t.len() && matches!(trimmed.last(), Some(b'.' | b'!' | b'?'))
}
