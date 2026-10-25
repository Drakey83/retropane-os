//! The responder over real FAT32 and exFAT images through
//! [`crate::fat::FatDevice`]: what Windows does in Explorer, end to end,
//! each card checked after.

use super::*;
use crate::fat::{Cards, FatDevice};
use rp_fat::AnyVolume;
use rp_fat::mkfs::{self, Fs, Options};
use rp_fat::node::{Node, ROOT as FROOT};
use rp_hal_api::{BlockDevice, BlockError};

struct Img {
    data: Vec<u8>,
    base: u32,
    /// The boot file's entry (sector, byte offset), as the guard records it.
    boot: (u32, usize),
}

struct Disk<'a>(&'a mut Img);

impl BlockDevice for Disk<'_> {
    fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
        let o = lba as usize * 512;
        let src = self
            .0
            .data
            .get(o..o + buf.len())
            .ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(src);
        Ok(())
    }
    fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
        let o = lba as usize * 512;
        // The guard's job, as the loader's: a write over the boot file's
        // entry is refused.
        let (bs, bo) = self.0.boot;
        if (lba..lba + (buf.len() / 512) as u32).contains(&bs) {
            let at = (bs - lba) as usize * 512 + bo;
            let now = &self.0.data[bs as usize * 512 + bo..bs as usize * 512 + bo + 32];
            if buf[at..at + 32] != *now {
                return Err(BlockError::ReadOnly);
            }
        }
        let dst = self
            .0
            .data
            .get_mut(o..o + buf.len())
            .ok_or(BlockError::OutOfRange)?;
        dst.copy_from_slice(buf);
        Ok(())
    }
}

struct TwoCards {
    imgs: [Img; 2],
    now: u32,
    name: Text,
    consent: Consent,
}

impl Cards for TwoCards {
    type Disk<'a> = Disk<'a>;
    fn stores(&self) -> u8 {
        2
    }
    fn disk(&mut self, s: u8) -> Option<(Disk<'_>, u32)> {
        let img = self.imgs.get_mut(usize::from(s))?;
        let base = img.base;
        Some((Disk(img), base))
    }
    fn kind(&self, _s: u8) -> Kind {
        Kind::Removable
    }
    fn writable(&self, _s: u8) -> bool {
        true
    }
    fn protected(&self, s: u8, sector: u32, offset: usize) -> bool {
        self.imgs[usize::from(s)].boot == (sector, offset)
    }
    fn boots_from(&mut self, s: u8) -> bool {
        s == 0
    }
    fn description(&self, s: u8) -> &'static str {
        if s == 0 { "SD Card" } else { "CF Card" }
    }
    fn format_consent(&mut self, _s: u8, _start: bool) -> Consent {
        self.now += 1000;
        self.consent
    }
    fn format(&mut self, _s: u8) -> Result<(), StoreError> {
        Ok(())
    }
    fn friendly_name(&self) -> Text {
        self.name
    }
    fn set_friendly_name(&mut self, name: &Text) {
        self.name = *name;
    }
    fn now_ms(&self) -> u32 {
        self.now
    }
}

fn image(fs: Fs, sectors: u32, cluster: u32) -> Img {
    let o = Options {
        fs: Some(fs),
        cluster_bytes: Some(cluster),
        align_sectors: None,
        label: "CARD",
        serial: 0xCAFE_0001,
    };
    let l = mkfs::plan(sectors, &o).unwrap();
    let mut img = Img {
        data: vec![0; sectors as usize * 512],
        base: l.part_start,
        boot: (0, 0),
    };
    mkfs::write(&mut Disk(&mut img), &l, &mut vec![0u8; 16 * 1024]).unwrap();
    // The boot file, written through the node layer, and its entry found.
    let base = img.base;
    let mut v = AnyVolume::mount(Disk(&mut img), base).unwrap();
    let s = &mut vec![0u8; 8192];
    let mut r = v.node_reserve(FROOT, 5, s).unwrap();
    v.node_write(&mut r, 0, b"HARET", s).unwrap();
    let loc = v
        .node_commit(
            &r,
            FROOT,
            &"haret.exe".encode_utf16().collect::<Vec<_>>(),
            false,
            (None, None),
            s,
        )
        .unwrap();
    let mut n = Node::default();
    v.node_stat(loc, &mut n).unwrap();
    let _ = v;
    img.boot = ((n.entry >> 4) as u32, (n.entry & 15) as usize * 32);
    img
}

fn clean(img: &mut Img) {
    let base = img.base;
    let mut v = AnyVolume::mount(Disk(img), base).unwrap();
    let (cap, _) = v.node_space(&mut vec![0u8; 8192]).unwrap();
    let mut seen = vec![0u8; (cap / 512 / 8) as usize + 16];
    let h = v.check(&mut seen, &mut vec![0u8; 8192]).unwrap();
    assert!(h.clean(), "{h:?}");
}

macro_rules! fat_pc {
    ($pc:ident, $fs:expr, $sectors:expr, $cluster:expr) => {
        let mut buf = vec![0u8; 16 * 1024];
        let mut slots = vec![Slot::default(); 1024];
        let mut index = vec![0u32; 2048];
        let mut scratch = vec![0u8; 16 * 1024];
        let cards = TwoCards {
            imgs: [
                image($fs, $sectors, $cluster),
                image(Fs::Fat16, 65_536, 2048),
            ],
            now: 0,
            name: Text::from_text("Axim X51v").unwrap(),
            consent: Consent::No,
        };
        let mut $pc = Pc {
            mtp: Mtp::new(identity(), &mut buf, &mut slots, &mut index),
            dev: FatDevice::new(cards, &mut scratch),
            pipe: TestPipe::default(),
            tid: 0,
        };
    };
}

fn on_each(mut f: impl FnMut(Fs, u32, u32, &str)) {
    f(Fs::Fat32, 131_072, 512, "FAT32");
    f(Fs::ExFat, 65_536, 4096, "exFAT");
}

#[test]
fn explorer_on_a_real_card_copies_reads_renames_moves_and_deletes() {
    on_each(|fsys, sectors, cluster, fs| {
        fat_pc!(pc, fsys, sectors, cluster);
        pc.open();
        assert_eq!(pc.list(u32::MAX).len(), 1, "{fs}: the boot file");
        // A folder, then a file in it with a name only UTF-16 holds.
        let a = pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), u32::MAX],
            Some(&object_info("Photos", 0, fmt::ASSOCIATION)),
        );
        assert_eq!(a.rc, rc::OK, "{fs}");
        let folder = a.params[2];
        let body: Vec<u8> = (0..40_000u32).map(|i| (i * 13 / 7) as u8).collect();
        let b = pc.op(
            op::SEND_OBJECT_INFO,
            &[store_id(0, true), folder],
            Some(&object_info("Zoë on the beach.jpg", 40_000, fmt::EXIF_JPEG)),
        );
        assert_eq!(b.rc, rc::OK, "{fs}");
        let h = b.params[2];
        assert_eq!(pc.op(op::SEND_OBJECT, &[], Some(&body)).rc, rc::OK, "{fs}");
        assert_eq!(pc.name_of(h), "Zoë on the beach.jpg", "{fs}");
        assert_eq!(pc.op(op::GET_OBJECT, &[h], None).data, body, "{fs}");
        assert_eq!(
            pc.op(op::GET_PARTIAL_OBJECT, &[h, 30_000, 100], None).data,
            body[30_000..30_100],
            "{fs}"
        );
        // Renamed, moved to the root, its handle kept.
        let mut nb = vec![0u8; 128];
        let mut w = Writer::new(&mut nb);
        w.text(&Text::from_text("Zoë, summer.jpg").unwrap());
        let n = w.len();
        assert_eq!(
            pc.op(
                op::SET_OBJECT_PROP_VALUE,
                &[h, u32::from(prop::OBJECT_FILE_NAME)],
                Some(&nb[..n])
            )
            .rc,
            rc::OK,
            "{fs}"
        );
        assert_eq!(
            pc.op(op::MOVE_OBJECT, &[h, store_id(0, true), 0], None).rc,
            rc::OK,
            "{fs}"
        );
        assert_eq!(pc.name_of(h), "Zoë, summer.jpg", "{fs}");
        assert!(pc.list(u32::MAX).contains(&h), "{fs}");
        assert_eq!(
            pc.op(op::GET_OBJECT, &[h], None).data,
            body,
            "{fs}: the same bytes after the move"
        );
        // Copied to the CF card (on the device), then the folder deleted.
        let c = pc.op(op::COPY_OBJECT, &[h, store_id(1, true), 0], None);
        assert_eq!(c.rc, rc::OK, "{fs}");
        assert_eq!(
            pc.op(op::GET_OBJECT, &[c.params[0]], None).data,
            body,
            "{fs}"
        );
        assert_eq!(
            pc.op(op::DELETE_OBJECT, &[folder, 0], None).rc,
            rc::OK,
            "{fs}"
        );
        // The boot file: read-only, hidden, and refused at once.
        let haret = pc.handle(u32::MAX, "haret.exe");
        let i = pc.op(op::GET_OBJECT_INFO, &[haret], None);
        assert_eq!(u16::from_le_bytes([i.data[6], i.data[7]]), 0x0001, "{fs}");
        assert_eq!(
            pc.op(op::DELETE_OBJECT, &[haret, 0], None).rc,
            rc::OBJECT_WRITE_PROTECTED,
            "{fs}"
        );
        assert_eq!(
            pc.op(
                op::SET_OBJECT_PROP_VALUE,
                &[haret, u32::from(prop::OBJECT_FILE_NAME)],
                Some(&nb[..n])
            )
            .rc,
            rc::OBJECT_WRITE_PROTECTED,
            "{fs}"
        );
        assert_eq!(pc.op(op::GET_OBJECT, &[haret], None).data, b"HARET", "{fs}");
        // Both cards consistent afterwards.
        let imgs = &mut pc.dev.cards().imgs;
        clean(&mut imgs[0]);
        clean(&mut imgs[1]);
    });
}

#[test]
fn a_cancelled_copy_on_a_real_card_leaves_no_file_and_no_lost_space() {
    on_each(|fsys, sectors, cluster, fs| {
        fat_pc!(pc, fsys, sectors, cluster);
        pc.open();
        let free = |pc: &mut Pc<'_, FatDevice<'_, TwoCards>>| {
            let a = pc.op(op::GET_STORAGE_INFO, &[store_id(0, true)], None);
            let mut r = Reader::new(&a.data);
            r.skip(6 + 8).unwrap();
            r.u64().unwrap()
        };
        let before = free(&mut pc);
        assert_eq!(
            pc.op(
                op::SEND_OBJECT_INFO,
                &[store_id(0, true), u32::MAX],
                Some(&object_info("half.bin", 100_000, fmt::UNDEFINED))
            )
            .rc,
            rc::OK,
            "{fs}"
        );
        pc.tid += 1;
        let tid = pc.tid;
        let all = packets(&container(
            container::DATA,
            op::SEND_OBJECT,
            tid,
            &[7u8; 100_000],
        ));
        pc.pipe
            .out
            .push_back(container(container::COMMAND, op::SEND_OBJECT, tid, &[]));
        for p in &all[..600] {
            pc.pipe.out.push_back(p.clone());
        }
        for _ in 0..2000 {
            pc.mtp.poll(&mut pc.dev, &mut pc.pipe);
        }
        // Unplugged.
        pc.mtp.reset(&mut pc.dev, &mut pc.pipe);
        pc.pipe.out.clear();
        pc.pipe.inp.clear();
        pc.open();
        assert!(
            pc.list(u32::MAX)
                .iter()
                .all(|&h| pc.name_of(h) != "half.bin"),
            "{fs}"
        );
        assert_eq!(free(&mut pc), before, "{fs}: the space came back");
        clean(&mut pc.dev.cards().imgs[0]);
    });
}

#[test]
fn a_power_cut_mid_copy_is_cleaned_at_the_next_mount() {
    on_each(|fsys, sectors, cluster, fs| {
        fat_pc!(pc, fsys, sectors, cluster);
        pc.open();
        assert_eq!(
            pc.op(
                op::SEND_OBJECT_INFO,
                &[store_id(0, true), u32::MAX],
                Some(&object_info("cut.bin", 50_000, fmt::UNDEFINED))
            )
            .rc,
            rc::OK,
            "{fs}"
        );
        // The power goes now: nothing is abandoned; the reservation stays on
        // the card as a temporary entry, which the next mount's recovery
        // finds.
        let img = &mut pc.dev.cards().imgs[0];
        let base = img.base;
        let mut v = AnyVolume::mount(Disk(img), base).unwrap();
        assert_eq!(v.node_recover().unwrap(), 1, "{fs}");
        let _ = v;
        clean(img);
    });
}

#[test]
fn storage_info_reads_the_real_card() {
    on_each(|fsys, sectors, cluster, fs| {
        fat_pc!(pc, fsys, sectors, cluster);
        pc.open();
        let a = pc.op(op::GET_STORAGE_INFO, &[store_id(0, true)], None);
        assert_eq!(a.rc, rc::OK, "{fs}");
        let mut r = Reader::new(&a.data);
        assert_eq!(r.u16().unwrap(), 4);
        assert_eq!(r.u16().unwrap(), 2);
        assert_eq!(r.u16().unwrap(), 0);
        let cap = r.u64().unwrap();
        let free = r.u64().unwrap();
        assert!(
            cap > 30 << 20 && free < cap && free > cap / 2,
            "{fs}: {cap} {free}"
        );
        r.u32().unwrap();
        assert_eq!(format!("{:?}", r.text().unwrap()), "SD Card");
        assert_eq!(
            format!("{:?}", r.text().unwrap()),
            "S0-0CAFE0001",
            "{fs}: the volume serial"
        );
    });
}
