# romdump

Unpacks Dell Axim X51v Windows Mobile ROM images (`.nb0`) and rebuilds the
driver DLLs as ordinary PE files, so they can be studied for
**interoperability** (see [`docs/CLEANROOM.md`](../../docs/CLEANROOM.md)).

- Reads the partition table: XIP (update loader), XIP (kernel), IMGFS.
- Walks IMGFS: files, modules, sections, long names.
- Decompresses LZX chunks (64 KiB window) via the `lzxd` crate.
- Rebuilds modules from Windows CE's `e32_rom`/`o32_rom` headers into PE files.
  It moves relocated data references back to the image base, and trims
  directories to the stored data, so the files load in standard PE tools.

Verified on LennySh WM6.1 A08 and L11 images: all 1,025 / 997 items extract.

## Usage

```bash
cargo run -p romdump -- info    local/rom/image.nb0
cargo run -p romdump -- list    local/rom/image.nb0
cargo run -p romdump -- extract local/rom/image.nb0 local/drivers ddi_2700g.dll libGLES_CL.dll
cargo run -p romdump -- extract local/rom/image.nb0 local/imgfs        # everything
cargo run -p romdump -- header  local/rom/image.nb0 ddi_2700g.dll
```

**Keep all input and output under `local/`** (it's git-ignored). ROM images and
extracted files are Dell/Microsoft property: use your own device's ROM, and never
commit or redistribute them.

## Not yet supported

- Extracting modules from the XIP partitions (only IMGFS is needed for the
  hardware drivers).
- Compression other than LZX (e.g. XPRESS, used by some other WM6 ROMs).
