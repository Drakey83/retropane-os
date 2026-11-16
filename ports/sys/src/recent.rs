//! The card's sectors read last, kept: a sector read again soon after is
//! answered from RAM.
//!
//! Why (R281, measured in the emulator 2026-11-16): with each folder's
//! names kept (`crate::names`), what is left of a game's look-ups and
//! listings is the first walk of each folder and the entry read when a
//! file is opened. The file system reads a folder one entry at a time and
//! reads the entry's sector again for each, sixteen times a sector; every
//! file opened in one folder reads that folder's sector again. On the X51v
//! each read command costs about a millisecond more than its data (run 2).
//!
//! Iterate, don't imitate (rule 0). An operating system keeps a buffer
//! cache of megabytes in front of the card, with read-ahead, and a port
//! to a small device either has none or copies one. Ours sits under the
//! file system as a block device and keeps only single sectors read last
//! (the folder and entry sectors that repeat; a file's data comes in
//! longer reads, which pass through), a few kilobytes chosen by count of
//! what a game's start re-reads, and drops any sector a write touches: it
//! can never answer with what the card no longer holds, whatever the file
//! system above it does.

use rp_hal_api::{BlockDevice, BlockError};

const SECTOR: usize = 512;
const EMPTY: u32 = u32::MAX;

/// `dev` with its last `K` single-sector reads kept.
pub struct Recent<D, const K: usize> {
    dev: D,
    lba: [u32; K],
    used: [u32; K],
    data: [[u8; SECTOR]; K],
    clock: u32,
    hits: u32,
}

impl<D: BlockDevice, const K: usize> Recent<D, K> {
    /// Nothing kept yet.
    pub const fn new(dev: D) -> Self {
        Self {
            dev,
            lba: [EMPTY; K],
            used: [0; K],
            data: [[0; SECTOR]; K],
            clock: 0,
            hits: 0,
        }
    }

    /// Reads answered from what was kept.
    #[must_use]
    pub fn hits(&self) -> u32 {
        self.hits
    }

    /// The device underneath.
    pub fn device(&mut self) -> &mut D {
        &mut self.dev
    }

    /// The device back.
    pub fn into_device(self) -> D {
        self.dev
    }

    fn tick(&mut self) -> u32 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }
}

impl<D: BlockDevice, const K: usize> BlockDevice for Recent<D, K> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        if buf.len() != SECTOR || K == 0 {
            return self.dev.read_blocks(lba, buf);
        }
        let now = self.tick();
        if let Some(k) = self.lba.iter().position(|&l| l == lba) {
            buf.copy_from_slice(&self.data[k]);
            self.used[k] = now;
            self.hits = self.hits.wrapping_add(1);
            return Ok(());
        }
        self.dev.read_blocks(lba, buf)?;
        // The one used longest ago makes way (an empty one first).
        let k = (0..K)
            .min_by_key(|&k| (self.lba[k] != EMPTY, now.wrapping_sub(self.used[k]) ^ u32::MAX))
            .unwrap_or(0);
        self.lba[k] = lba;
        self.used[k] = now;
        self.data[k].copy_from_slice(buf);
        Ok(())
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        // Dropped whether or not the write went through: after a failed
        // one the card's sectors there are not known.
        let end = u64::from(lba) + (buf.len() / SECTOR) as u64;
        for l in &mut self.lba {
            if *l != EMPTY && u64::from(*l) >= u64::from(lba) && u64::from(*l) < end {
                *l = EMPTY;
            }
        }
        self.dev.write_blocks(lba, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card of 64 sectors that counts its reads.
    struct Card {
        bytes: Vec<u8>,
        reads: u32,
        fail_next_write: bool,
    }

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            self.reads += 1;
            let o = lba as usize * SECTOR;
            let src = self.bytes.get(o..o + buf.len()).ok_or(BlockError::OutOfRange)?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            let o = lba as usize * SECTOR;
            // A failed write that still changed the card part way.
            self.bytes[o..o + SECTOR].copy_from_slice(&buf[..SECTOR]);
            if std::mem::take(&mut self.fail_next_write) {
                return Err(BlockError::Device);
            }
            self.bytes[o..o + buf.len()].copy_from_slice(buf);
            Ok(())
        }
    }

    fn card() -> Card {
        Card {
            bytes: (0..64 * SECTOR).map(|i| (i / SECTOR * 7 + i % 251) as u8).collect(),
            reads: 0,
            fail_next_write: false,
        }
    }

    fn sector(r: &mut Recent<Card, 4>, lba: u32) -> Vec<u8> {
        let mut b = vec![0u8; SECTOR];
        r.read_blocks(lba, &mut b).unwrap();
        b
    }

    #[test]
    fn a_sector_read_again_comes_from_ram_and_the_oldest_makes_way() {
        let mut r: Recent<Card, 4> = Recent::new(card());
        let first = sector(&mut r, 5);
        assert_eq!(sector(&mut r, 5), first);
        assert_eq!((r.device().reads, r.hits()), (1, 1));
        for l in [6, 7, 8] {
            sector(&mut r, l);
        }
        sector(&mut r, 5); // 5 used last: 6 is the oldest now
        sector(&mut r, 9); // 6 makes way
        let reads = r.device().reads;
        sector(&mut r, 5);
        sector(&mut r, 9);
        assert_eq!(r.device().reads, reads, "5 and 9 kept");
        sector(&mut r, 6);
        assert_eq!(r.device().reads, reads + 1, "6 was the one dropped");
    }

    #[test]
    fn longer_reads_pass_through_and_a_write_is_never_hidden() {
        let mut r: Recent<Card, 4> = Recent::new(card());
        for l in 10..14 {
            sector(&mut r, l);
        }
        // A longer read goes to the card each time.
        let mut two = vec![0u8; 2 * SECTOR];
        r.read_blocks(10, &mut two).unwrap();
        r.read_blocks(10, &mut two).unwrap();
        assert_eq!(r.device().reads, 6);
        // A write over two kept sectors: both read back as written.
        r.write_blocks(11, &[0xAB; 2 * SECTOR]).unwrap();
        assert_eq!(sector(&mut r, 11), vec![0xAB; SECTOR]);
        assert_eq!(sector(&mut r, 12), vec![0xAB; SECTOR]);
        // A write that failed part way: what the card holds, not what was kept.
        r.device().fail_next_write = true;
        assert!(r.write_blocks(10, &[0xCD; SECTOR]).is_err());
        assert_eq!(sector(&mut r, 10), vec![0xCD; SECTOR]);
        // The sectors around it stay kept.
        let reads = r.device().reads;
        sector(&mut r, 13);
        assert_eq!(r.device().reads, reads);
    }

    #[test]
    fn a_failed_read_keeps_nothing() {
        let mut r: Recent<Card, 4> = Recent::new(card());
        let mut b = vec![0u8; SECTOR];
        assert!(r.read_blocks(1000, &mut b).is_err());
        assert!(r.read_blocks(1000, &mut b).is_err());
        assert_eq!(r.hits(), 0);
    }
}
