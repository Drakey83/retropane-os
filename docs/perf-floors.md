# Performance floors (the ratchet: docs/design/perf-ratchet.md)

Written by `cargo xtask perf noise N` and `cargo xtask perf --update`; a
slower floor needs the owner's approval in its last column. Every number is
a time on the X51v (lower is better): the probe's ns an iteration, the test
images' microseconds. The tolerance is the noise measured across the runs
that set the floor (at least one unit of the number).

| Number | Floor | Tolerance % | Set by | Approved |
|---|---|---|---|---|
| `b7d_convert_32_to_16_bits_480_x_80` | 1206 | 0.6633 | noise 5 runs, refloored |  |
| `b7d_convert_32_to_16_bits_full_screen` | 9634 | 0.0830 | noise 5 runs, refloored |  |
| `b7d_copy_16_to_16_bits_480_x_80` | 630 | 1.2698 | noise 5 runs, refloored |  |
| `b7d_copy_16_to_16_bits_a_forward_copy_full_screen` | 5032 | 0.1592 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `b7d_copy_32_to_32_bits_half_screen_b6_s_control` | 5065 | 0.3972 | noise 5 runs, refloored |  |
| `b7d_dither_add_at_32_bits_4_blits_blend_0_full_screen` | 18885 | 0.0424 | noise 5 runs, refloored |  |
| `b7d_make_the_32_bit_picture_stride_0_copy_full_screen` | 10048 | 0.1419 | noise 5 runs, refloored |  |
| `b8_dither_add_480_x_80.four_blits_from_pattern_rows` | 2366 | 0.3381 | noise 5 runs, refloored |  |
| `b8_dither_add_480_x_80.one_blit_from_the_full_screen_pattern` | 2362 | 0.3387 | noise 5 runs, refloored |  |
| `b8_dither_add_full_screen.four_blits_from_pattern_rows` | 18886 | 0.0424 | noise 5 runs, refloored |  |
| `b8_dither_add_full_screen.one_blit_from_the_full_screen_pattern` | 18880 | 0.0424 | noise 5 runs, refloored |  |
| `b9e_full_screen_us_each.16_bit_copy` | 5033 | 0.1592 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `b9e_full_screen_us_each.1_from_480_x_640_32_bit` | 9634 | 0.0830 | noise 5 runs, refloored |  |
| `b9e_full_screen_us_each.2x_upscale_from_240_x_320_32_bit` | 18688 | 0.0428 | noise 5 runs, refloored |  |
| `b9e_full_screen_us_each.8_bpp_through_the_palette` | 15800 | 0.0506 | noise 5 runs, refloored |  |
| `ft_alone.2d_10_copies` | 71915 | 0.1264 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `ft_alone.3d` | 21970 | 0.2687 | 1792708713 |  |
| `probe.cache_walk_16k` | 14028 | 4.3868 | noise 5 runs, refloored |  |
| `probe.cache_walk_24k` | 21018 | 2.9279 | noise 5 runs, refloored |  |
| `probe.cache_walk_256k` | 1910076 | 0.0322 | noise 5 runs, refloored |  |
| `probe.cache_walk_28k` | 24503 | 2.5115 | noise 5 runs, refloored |  |
| `probe.cache_walk_32k` | 47057 | 1.0647 | 1792708713 |  |
| `probe.cache_walk_40k` | 298497 | 0.2080 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `probe.cache_walk_48k` | 357933 | 0.1719 | noise 5 runs, refloored |  |
| `probe.cache_walk_4k` | 3560 | 17.2861 | noise 5 runs, refloored |  |
| `probe.cache_walk_64k` | 478249 | 0.1287 | noise 5 runs, refloored |  |
| `probe.cache_walk_8k` | 7050 | 8.7289 | noise 5 runs, refloored |  |
| `probe.chase_sdram_256k` | 2013192 | 0.0306 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `probe.chase_sram_256k` | 454306 | 0.1355 | noise 5 runs, refloored |  |
| `probe.copy_16k` | 43547 | 1.4132 | noise 5 runs, refloored |  |
| `probe.copy_1k` | 2758 | 22.3127 | noise 5 runs, refloored |  |
| `probe.copy_1m` | 9630153 | 0.0064 | noise 5 runs, refloored |  |
| `probe.copy_256b` | 711 | 86.5520 | noise 5 runs, refloored |  |
| `probe.copy_256k` | 2411076 | 0.0255 | noise 5 runs, refloored |  |
| `probe.copy_4k` | 10944 | 5.6230 | noise 5 runs, refloored |  |
| `probe.copy_64b` | 266 | 231.3476 | noise 5 runs, refloored |  |
| `probe.copy_64k` | 606096 | 0.1015 | noise 5 runs, refloored |  |
| `probe.copy_pld_128_256k` | 2409306 | 0.0255 | noise 5 runs, refloored |  |
| `probe.copy_pld_192_256k` | 2409999 | 0.0255 | noise 5 runs, refloored |  |
| `probe.copy_pld_256_256k` | 2410845 | 0.0255 | noise 5 runs, refloored |  |
| `probe.copy_pld_32_256k` | 3401691 | 0.1501 | 1792708713 |  |
| `probe.copy_pld_64_256k` | 2739845 | 0.0225 | noise 5 runs, refloored |  |
| `probe.copy_pld_96_256k` | 2408999 | 0.0255 | noise 5 runs, refloored |  |
| `probe.copy_pld_none_256k` | 5600923 | 0.0110 | noise 5 runs, refloored |  |
| `probe.cpu_unit_alone` | 19276 | 3.1925 | noise 5 runs, refloored |  |
| `probe.cpu_unit_during_dma` | 22149 | 2.7784 | noise 5 runs, refloored |  |
| `probe.dma_copy_128k` | 2923845 | 0.0210 | noise 5 runs, refloored |  |
| `probe.dma_copy_128k_b16_word` | 4541999 | 0.0129 | 1792708713 |  |
| `probe.dma_copy_128k_b8_word` | 8250461 | 0.0218 | 1792708713 |  |
| `probe.dma_copy_128k_paced` | 2898229 | 0.2966 | noise 5 runs, refloored |  |
| `probe.dot48_mixer_shape_block` | 16499 | 3.7298 | noise 5 runs, refloored |  |
| `probe.dot48_plain_flat_block` | 15166 | 4.0577 | noise 5 runs, refloored |  |
| `probe.dot48_wmmx_block` | 11296 | 5.4478 | noise 5 runs, refloored |  |
| `probe.fb_pattern_vga` | 14478974 | 0.0043 | noise 5 runs, refloored |  |
| `probe.fb_solid_fill_vga` | 14473846 | 0.0043 | noise 5 runs, refloored |  |
| `probe.frames_alloc_free_split` | 1234 | 49.8691 | noise 5 runs, refloored |  |
| `probe.frames_fill_drain_1024` | 417291 | 0.1430 | 1792708713 |  |
| `probe.gfx_blend_fill_vga` | 13607537 | 0.0045 | 1792708713 |  |
| `probe.gfx_blend_reference_vga` | 20547537 | 0.0030 | 1792708713 |  |
| `probe.gfx_fill_vga` | 1568820 | 0.0392 | noise 5 runs, refloored |  |
| `probe.gfx_flush_clock` | 114220 | 0.5388 | noise 5 runs, refloored |  |
| `probe.gfx_flush_vga` | 17794051 | 0.0035 | 1792708713 | owner 2026-10-22 (option A, after B: ADR-0001 "Why, and what was fixed") |
| `probe.gfx_glass_vga` | 15344460 | 0.0040 | 1792708713 |  |
| `probe.gfx_icon_64` | 107738 | 0.5712 | noise 5 runs, refloored |  |
| `probe.gfx_photo_row_480` | 30183 | 1.8940 | 1792708713 |  |
| `probe.lz4_compress_noise` | 54210 | 0.8820 | 1792708713 |  |
| `probe.lz4_compress_page` | 182980 | 0.3051 | 1792708713 |  |
| `probe.lz4_compress_text` | 144345 | 0.4092 | 1792708713 |  |
| `probe.lz4_decompress_noise` | 20278 | 3.0347 | noise 5 runs, refloored |  |
| `probe.lz4_decompress_page` | 134110 | 0.4111 | 1792708713 |  |
| `probe.lz4_decompress_text` | 99788 | 0.6052 | 1792708713 |  |
| `probe.memcpy_128k` | 1207537 | 0.0510 | noise 5 runs, refloored |  |
| `probe.memcpy_64k` | 605522 | 0.1016 | noise 5 runs, refloored |  |
| `probe.memcpy_naive_64k` | 2189092 | 0.0281 | noise 5 runs, refloored |  |
| `probe.memcpy_sdram_to_sram_64k` | 396722 | 0.1543 | 1792708713 |  |
| `probe.memcpy_sram_64k` | 229199 | 0.2685 | noise 5 runs, refloored |  |
| `probe.memset_64k` | 173953 | 0.3538 | noise 5 runs, refloored |  |
| `probe.memset_sram_64k` | 137953 | 0.4461 | noise 5 runs, refloored |  |
| `probe.plain_avg_128k` | 3139384 | 0.0196 | noise 5 runs, refloored |  |
| `probe.plain_avg_8k` | 36240 | 1.6981 | noise 5 runs, refloored |  |
| `probe.plain_mix_128k` | 3860768 | 0.0159 | noise 5 runs, refloored |  |
| `probe.plain_mix_8k` | 85431 | 0.7203 | noise 5 runs, refloored |  |
| `probe.sd_read_256_blocks` | 67750153 | 0.0015 | noise 5 runs, refloored |  |
| `probe.sd_read_256_blocks_dma` | 15952000 | 0.0107 | noise 5 runs, refloored |  |
| `probe.sd_read_4_blocks` | 1674653 | 0.0531 | noise 5 runs, refloored |  |
| `probe.sd_read_64_blocks` | 17180691 | 0.0112 | noise 5 runs, refloored |  |
| `probe.sd_read_64_blocks_dma` | 4494999 | 0.0213 | noise 5 runs, refloored |  |
| `probe.sd_read_block` | 887653 | 0.0871 | noise 5 runs, refloored |  |
| `probe.timer_read` | 846 | 72.7405 | noise 5 runs, refloored |  |
| `probe.tlb_fit_16` | 270 | 227.9202 | noise 5 runs, refloored |  |
| `probe.tlb_fit_16_l1sram` | 270 | 227.9202 | noise 5 runs, refloored |  |
| `probe.tlb_spill_60` | 19964 | 3.0825 | noise 5 runs, refloored |  |
| `probe.tlb_spill_60_l1sram` | 3862 | 15.9344 | noise 5 runs, refloored |  |
| `probe.wmmx_avg_128k` | 2138076 | 0.0288 | noise 5 runs, refloored |  |
| `probe.wmmx_avg_8k` | 12812 | 4.8032 | noise 5 runs, refloored |  |
| `probe.wmmx_copy_256k` | 2408691 | 0.0255 | noise 5 runs, refloored |  |
| `probe.wmmx_memcpy_256k` | 2409768 | 0.0255 | noise 5 runs, refloored |  |
| `probe.wmmx_mix_128k` | 1808691 | 0.0426 | noise 5 runs, refloored |  |
| `probe.wmmx_mix_8k` | 12807 | 4.8051 | noise 5 runs, refloored |  |
