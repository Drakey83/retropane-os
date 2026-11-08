# Performance floors (the ratchet: docs/design/perf-ratchet.md)

Written by `cargo xtask perf noise N` and `cargo xtask perf --update`; a
slower floor needs the owner's approval in its last column. Every number is
a time on the X51v (lower is better): the probe's ns an iteration, the test
images' microseconds. The tolerance is the noise measured across the runs
that set the floor (at least one unit of the number).

| Number | Floor | Tolerance % | Set by | Approved |
|---|---|---|---|---|
| `b7d_convert_32_to_16_bits_480_x_80` | 1206 | 0.6633 | noise 15 runs, refloored |  |
| `b7d_convert_32_to_16_bits_full_screen` | 9634 | 0.0830 | noise 15 runs, refloored |  |
| `b7d_copy_16_to_16_bits_480_x_80` | 629 | 1.2719 | noise 15 runs, refloored |  |
| `b7d_copy_16_to_16_bits_a_forward_copy_full_screen` | 5033 | 0.1819 | noise 15 runs, refloored |  |
| `b7d_copy_32_to_32_bits_half_screen_b6_s_control` | 5067 | 0.3669 | noise 15 runs, refloored |  |
| `b7d_dither_add_at_32_bits_4_blits_blend_0_full_screen` | 18885 | 0.0424 | noise 15 runs, refloored |  |
| `b7d_make_the_32_bit_picture_stride_0_copy_full_screen` | 10056 | 0.0796 | noise 15 runs, refloored |  |
| `b8_dither_add_480_x_80.four_blits_from_pattern_rows` | 2366 | 0.3381 | noise 15 runs, refloored |  |
| `b8_dither_add_480_x_80.one_blit_from_the_full_screen_pattern` | 2361 | 0.3388 | noise 5 runs, run-1793284682 |  |
| `b8_dither_add_full_screen.four_blits_from_pattern_rows` | 18886 | 0.0424 | noise 15 runs, refloored |  |
| `b8_dither_add_full_screen.one_blit_from_the_full_screen_pattern` | 18880 | 0.0424 | noise 15 runs, refloored |  |
| `b9e_full_screen_us_each.16_bit_copy` | 5033 | 0.2078 | noise 15 runs, refloored |  |
| `b9e_full_screen_us_each.1_from_480_x_640_32_bit` | 9634 | 0.0830 | noise 15 runs, refloored |  |
| `b9e_full_screen_us_each.2x_upscale_from_240_x_320_32_bit` | 18688 | 0.0428 | noise 15 runs, refloored |  |
| `b9e_full_screen_us_each.8_bpp_through_the_palette` | 15800 | 0.0506 | noise 15 runs, refloored |  |
| `ft_alone.2d_10_copies` | 71880 | 0.1736 | noise 5 runs, run-1793284682 |  |
| `ft_alone.3d` | 21471 | 0.1745 | noise 15 runs, refloored |  |
| `probe.cache_walk_16k` | 14033 | 4.3853 | noise 15 runs, refloored |  |
| `probe.cache_walk_24k` | 21010 | 2.9290 | noise 15 runs, refloored |  |
| `probe.cache_walk_256k` | 1910153 | 0.0322 | noise 15 runs, refloored |  |
| `probe.cache_walk_28k` | 24503 | 2.5115 | noise 15 runs, refloored |  |
| `probe.cache_walk_32k` | 46807 | 1.3147 | noise 15 runs, refloored |  |
| `probe.cache_walk_40k` | 298534 | 0.2085 | the last session's median (4 runs, run-1793368325 to run-1793371107) | owner 2026-11-07 (slower floor approved, decisions-2026-11-06 item 8; last session run-1793368325 to run-1793371107) |
| `probe.cache_walk_48k` | 357846 | 0.1720 | noise 15 runs, refloored | owner 2026-11-07 (slower floor approved, decisions-2026-11-06 item 8; last session run-1793368325 to run-1793371107) |
| `probe.cache_walk_4k` | 3558 | 17.2958 | noise 5 runs, run-1793284682 |  |
| `probe.cache_walk_64k` | 478076 | 0.1287 | noise 5 runs, run-1793284682 |  |
| `probe.cache_walk_8k` | 7050 | 8.7289 | noise 5 runs, run-1793284682 |  |
| `probe.chase_sdram_256k` | 1282999 | 0.0480 | noise 15 runs, refloored | benchmark changed (0b2e1be2): one word load a hop, not a gain (R97, the owner's ruling 6, 2026-10-29) |
| `probe.chase_sram_256k` | 345691 | 0.1780 | noise 15 runs, refloored | benchmark changed (0b2e1be2): one word load a hop, not a gain (R97, the owner's ruling 6, 2026-10-29) |
| `probe.copy_16k` | 43533 | 1.4136 | noise 15 runs, refloored |  |
| `probe.copy_1k` | 2740 | 22.4593 | noise 5 runs, run-1793284682 |  |
| `probe.copy_1m` | 9629846 | 0.0072 | noise 15 runs, refloored |  |
| `probe.copy_256b` | 705 | 87.2886 | noise 15 runs, refloored |  |
| `probe.copy_256k` | 2410537 | 0.0255 | noise 15 runs, refloored |  |
| `probe.copy_4k` | 10927 | 5.6318 | noise 5 runs, run-1793284682 |  |
| `probe.copy_64b` | 240 | 256.4103 | noise 15 runs, refloored |  |
| `probe.copy_64k` | 605749 | 0.1016 | noise 15 runs, refloored |  |
| `probe.copy_pld_128_256k` | 2409153 | 0.0255 | noise 15 runs, refloored |  |
| `probe.copy_pld_192_256k` | 2409999 | 0.0255 | noise 15 runs, refloored |  |
| `probe.copy_pld_256_256k` | 2410614 | 0.0255 | noise 15 runs, refloored |  |
| `probe.copy_pld_32_256k` | 3401614 | 0.3137 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0181%) |
| `probe.copy_pld_64_256k` | 2739692 | 0.0225 | noise 5 runs, run-1793284682 |  |
| `probe.copy_pld_96_256k` | 2408923 | 0.0255 | noise 15 runs, refloored |  |
| `probe.copy_pld_none_256k` | 5600615 | 0.0110 | noise 15 runs, refloored |  |
| `probe.cpu_unit_alone` | 19274 | 3.1928 | noise 15 runs, refloored |  |
| `probe.cpu_unit_during_dma` | 22132 | 2.7805 | noise 5 runs, run-1793284682 |  |
| `probe.dma_copy_128k` | 2923460 | 0.0210 | noise 5 runs, run-1793284682 |  |
| `probe.dma_copy_128k_b16_word` | 4541922 | 0.0135 | noise 5 runs, run-1793284682 |  |
| `probe.dma_copy_128k_b32_w0` | 2906922 | 0.0212 | noise 15 runs, refloored |  |
| `probe.dma_copy_128k_b8_word` | 8248845 | 0.0075 | noise 15 runs, refloored |  |
| `probe.dma_copy_128k_paced` | 2902383 | 0.2907 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0212%) |
| `probe.dot48_mixer_shape_block` | 15982 | 3.8505 | noise 15 runs, refloored |  |
| `probe.dot48_plain_flat_block` | 15059 | 4.0865 | noise 15 runs, refloored |  |
| `probe.dot48_wmmx_block` | 11294 | 5.4488 | noise 15 runs, refloored |  |
| `probe.fb_pattern_vga` | 14478358 | 0.0053 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0043%) |
| `probe.fb_solid_fill_vga` | 14473846 | 0.0043 | noise 15 runs, refloored |  |
| `probe.frames_alloc_free_split` | 1228 | 50.1128 | noise 15 runs, refloored |  |
| `probe.frames_fill_drain_1024` | 417322 | 0.1475 | noise 15 runs, refloored | re-measured, not approved slower: code placement, the old probe gave the same (R97, ruling 6, 2026-10-29) |
| `probe.gfx_blend_fill_vga` | 13607692 | 0.0513 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0045%) |
| `probe.gfx_blend_reference_vga` | 20547537 | 0.8271 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0030%) |
| `probe.gfx_fill_vga` | 1568615 | 0.0392 | noise 15 runs, refloored |  |
| `probe.gfx_flush_clock` | 114233 | 1.6405 | noise 15 runs, refloored |  |
| `probe.gfx_flush_vga` | 17795589 | 0.1727 | noise 5 runs, run-1793284682 | tolerance between sessions: four standard deviations of the medians of the five sessions in local/perf, 2026-10-22 to 10-29 (17,792,512 to 17,808,717; sd 7,683), the tool's own 4-sd rule; five back-to-back runs (0.0035%) can't see the drift between boots and builds (code placement, R97). Floor unchanged; not approved slower (R97, ruling 6, 2026-10-29) |
| `probe.gfx_glass_vga` | 15340768 | 0.1718 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0040%) |
| `probe.gfx_icon_64` | 107738 | 0.5712 | noise 15 runs, refloored |  |
| `probe.gfx_photo_row_480` | 30183 | 2.0388 | noise 15 runs, refloored |  |
| `probe.lz4_compress_noise` | 54172 | 1.1360 | noise 5 runs, run-1793284682 |  |
| `probe.lz4_compress_page` | 182961 | 0.3363 | noise 15 runs, refloored |  |
| `probe.lz4_compress_text` | 144307 | 0.4264 | noise 15 runs, refloored |  |
| `probe.lz4_decompress_noise` | 20274 | 3.0353 | noise 15 runs, refloored |  |
| `probe.lz4_decompress_page` | 134110 | 0.4589 | noise 15 runs, refloored |  |
| `probe.lz4_decompress_text` | 99788 | 0.6167 | noise 5 runs, run-1793284682 |  |
| `probe.memcpy_128k` | 1207152 | 0.0510 | noise 5 runs, run-1793284682 |  |
| `probe.memcpy_64k` | 605368 | 0.1017 | noise 15 runs, refloored |  |
| `probe.memcpy_naive_64k` | 2188615 | 0.0294 | noise 15 runs, refloored |  |
| `probe.memcpy_sdram_to_sram_64k` | 397768 | 0.7183 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.1547%) |
| `probe.memcpy_sram_64k` | 228907 | 0.2688 | noise 5 runs, run-1793284682 |  |
| `probe.memset_64k` | 173907 | 0.3539 | noise 15 runs, refloored |  |
| `probe.memset_sram_64k` | 137892 | 0.4463 | noise 15 runs, refloored |  |
| `probe.mul_chain_k1` | 8192 | 7.5120 | noise 15 runs, refloored |  |
| `probe.mul_chain_k15` | 8192 | 7.5120 | noise 15 runs, refloored |  |
| `probe.mul_chain_k16` | 9807 | 6.2750 | noise 15 runs, refloored |  |
| `probe.mul_chain_k27` | 9807 | 6.2750 | noise 15 runs, refloored |  |
| `probe.mul_chain_k32` | 11384 | 5.4057 | noise 15 runs, refloored |  |
| `probe.plain_avg_128k` | 3139306 | 0.0196 | noise 15 runs, refloored |  |
| `probe.plain_avg_8k` | 36235 | 1.6983 | noise 5 runs, run-1793284682 |  |
| `probe.plain_mix_128k` | 3860537 | 0.0159 | noise 15 runs, refloored |  |
| `probe.plain_mix_8k` | 85398 | 0.7206 | noise 15 runs, refloored |  |
| `probe.sd_read_256_blocks` | 67749383 | 0.0025 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0019%) |
| `probe.sd_read_256_blocks_dma` | 15950306 | 0.0172 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0096%) |
| `probe.sd_read_4_blocks` | 1673672 | 0.0814 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0368%) |
| `probe.sd_read_64_blocks` | 17179614 | 0.0089 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0065%) |
| `probe.sd_read_64_blocks_dma` | 4493460 | 0.0624 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0215%) |
| `probe.sd_read_block` | 886634 | 0.1489 | noise 5 runs, run-1793284682 | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.1037%) |
| `probe.timer_read` | 846 | 72.7405 | noise 15 runs, refloored |  |
| `probe.tlb_fit_16` | 253 | 243.2350 | noise 15 runs, refloored |  |
| `probe.tlb_fit_16_l1sram` | 253 | 243.2350 | noise 15 runs, refloored |  |
| `probe.tlb_spill_60` | 20057 | 3.0682 | noise 15 runs, refloored |  |
| `probe.tlb_spill_60_l1sram` | 3847 | 15.9965 | noise 15 runs, refloored |  |
| `probe.wmmx_avg_128k` | 2138076 | 0.0288 | noise 5 runs, run-1793284682 |  |
| `probe.wmmx_avg_8k` | 12812 | 4.8032 | noise 15 runs, refloored |  |
| `probe.wmmx_copy_256k` | 2408537 | 0.0256 | noise 15 runs, refloored |  |
| `probe.wmmx_memcpy_256k` | 2409306 | 0.0255 | noise 15 runs, refloored |  |
| `probe.wmmx_mix_128k` | 1807537 | 0.1342 | noise 15 runs, refloored | tolerance between sessions: 4 sd of 11 session medians (perf sessions, 2026-11-07, item 8, R109's drift-type list; was 0.0340%) |
| `probe.wmmx_mix_8k` | 12792 | 4.8107 | noise 15 runs, refloored |  |
| `tr_touch_replay.frame_and_queueing_median` | 380 | 3.1579 | runs run-1793369429, run-1793370352, run-1793371107 (today-0016 to -0018, R115 part 2) | owner 2026-10-30 ("Approve the new limits": tear-free touch, the flip in the blanking or the copy behind the scan) |
| `tr_touch_replay.frame_and_queueing_worst` | 56652 | 1.8018 | noise 15 runs, refloored | owner 2026-10-30 ("Approve the new limits": tear-free touch, the flip in the blanking or the copy behind the scan) |
| `tr_touch_replay.landing_to_verdict_median` | 8924 | 0.0448 | runs run-1793369429, run-1793370352, run-1793371107 (today-0016 to -0018, R115 part 2) | owner 2026-10-30 ("Approve the new limits": tear-free touch, the flip in the blanking or the copy behind the scan); owner 2026-11-07 (slower floor approved, decisions-2026-11-06 item 8; last session run-1793368325 to run-1793371107) |
| `tr_touch_replay.landing_to_verdict_worst` | 15884 | 0.0504 | noise 5 runs, run-1793284682 |  |
| `tr_touch_replay.pen_down_period_median` | 2600 | 34.1888 | noise 15 runs, refloored |  |
| `tr_touch_replay.pen_down_period_worst` | 12724 | 0.3832 | the last session's median (4 runs, run-1793368325 to run-1793371107) | owner 2026-11-07 (slower floor approved, decisions-2026-11-06 item 8; last session run-1793368325 to run-1793371107) |
| `tr_touch_replay.queue_to_flip_median` | 5088 | 0.1572 | noise 15 runs, refloored |  |
| `tr_touch_replay.queue_to_flip_worst` | 18032 | 0.1975 | noise 15 runs, refloored |  |
| `tr_touch_replay.touch_to_flush_median` | 16204 | 0.0494 | runs run-1793369429, run-1793370352, run-1793371107 (today-0016 to -0018, R115 part 2) | owner 2026-10-30 ("Approve the new limits": tear-free touch, the flip in the blanking or the copy behind the scan) |
| `tr_touch_replay.touch_to_flush_worst` | 43624 | 14.3041 | runs run-1793369429, run-1793370352, run-1793371107 (today-0016 to -0018, R115 part 2) | owner 2026-10-30 ("Approve the new limits": tear-free touch, the flip in the blanking or the copy behind the scan) |
