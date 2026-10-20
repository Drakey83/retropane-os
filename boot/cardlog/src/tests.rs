use super::*;
use rp_hal_api::BlockError;

/// A card in memory that counts its writes and can refuse them.
struct Mem {
    data: Vec<u8>,
    writes: usize,
    refuse: bool,
}

impl BlockDevice for Mem {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        let o = lba as usize * SECTOR;
        buf.copy_from_slice(&self.data[o..o + buf.len()]);
        Ok(())
    }

    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        if self.refuse {
            return Err(BlockError::Device);
        }
        let o = lba as usize * SECTOR;
        self.data[o..o + buf.len()].copy_from_slice(buf);
        self.writes += 1;
        Ok(())
    }
}

fn log(sectors: u32) -> CardLog<Mem> {
    let dev = Mem {
        data: vec![0; (8 + sectors as usize) * SECTOR],
        writes: 0,
        refuse: false,
    };
    CardLog::new(dev, Reserved { lba: 8, sectors })
}

/// The file's text as the PC reads it: up to the first zero byte.
fn text(l: &CardLog<Mem>) -> String {
    let file = &l.dev.data[8 * SECTOR..];
    let end = file.iter().position(|&b| b == 0).unwrap_or(file.len());
    String::from_utf8(file[..end].to_vec()).unwrap()
}

#[test]
fn every_finished_line_is_on_the_card_at_once() {
    let mut l = log(4);
    l.write(b"first ");
    assert_eq!(l.dev.writes, 0, "a line in pieces waits for its end");
    l.write(b"line\n");
    assert_eq!(text(&l), "first line\n");
    l.write(b"second\n");
    assert_eq!(text(&l), "first line\nsecond\n");
    assert_eq!(l.dev.writes, 2);
    // Nothing outside the file is touched.
    assert!(l.dev.data[..8 * SECTOR].iter().all(|&b| b == 0));
}

#[test]
fn lines_across_sectors_and_a_full_file() {
    let mut l = log(3);
    let line = b"0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopq\n"; // 64
    let mut want = String::new();
    for _ in 0..20 {
        l.write(line);
        want.push_str(core::str::from_utf8(line).unwrap());
    }
    // 1280 bytes: two whole sectors and a part.
    assert_eq!(text(&l), want);
    assert!(!l.full());
    for _ in 0..10 {
        l.write(line);
    }
    // 1920 bytes asked, 1536 fit: the rest counted, never written past.
    assert!(l.full());
    assert_eq!(l.dropped, 1920 - 1536);
    assert_eq!(text(&l).len(), 1536);
    assert_eq!(l.dev.data.len(), 11 * SECTOR);
}

#[test]
fn a_refused_write_is_counted_and_the_sector_written_with_the_next_line() {
    let mut l = log(2);
    l.dev.refuse = true;
    l.write(b"lost for now\n");
    assert_eq!(l.failed, 1);
    l.dev.refuse = false;
    l.write(b"next\n");
    assert_eq!(text(&l), "lost for now\nnext\n");
}

#[test]
fn numbers_follow_the_highest_of_this_image_only() {
    let names: [&[u8]; 6] = [
        b"gpubench-0001.txt",
        b"gpubench-0012.txt",
        b"gpubench-x.txt",
        b"noise-0099.txt",
        b"gpubench-0003.txt",
        b"gpubench-.txt",
    ];
    assert_eq!(next_number("gpubench", names.iter().copied()), 13);
    assert_eq!(next_number("mbx3d", names.iter().copied()), 1);
    let mut out = [0u8; 64];
    let n = path("gpubench", 13, &mut out);
    assert_eq!(&out[..n], b"rplog/gpubench-0013.txt");
    let n = path("gpubench", 123_456, &mut out);
    assert_eq!(&out[..n], b"rplog/gpubench-123456.txt");
}
