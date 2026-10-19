# Mutant gaps (quality audit C5)

The rule: every guard gets a mutant (`xtask/src/mutants.rs`), so its test is
known to bite. This list is every test whose name says it checks a guard
(refuses, rejects, never, bounded, hostile, malformed...) that no mutant
names yet: each is a guard whose mutant is still to be written, or a test
already covered by a mutant aimed at another test of the same guard (to be
checked one by one). Generated 2026-10-19 by cross-referencing the test
names against the mutants (`tools`-free: a short script); regenerate after
adding mutants.

**1074 tests, 107 test filters named by mutants, 172 guard-like tests without a mutant naming them.**

**Retired 2026-10-19 late night, as near-equivalent in the host model:**
"mixer: wakes only when the mix is about to run out" (the low-water mark
dropped from the wake's timing). Killed until ADR-0032 A6-B's follow-up
counted a stream dry only when its dry stretch plays. Since then no test
shows it: a stream's ring holds a write's slack (~32-45 ms) of
written-but-unmixed sound, and the mixer wakes early on its own whenever
its mix reaches the data's end, which together give a program the time the
low-water mark was added for. Not even a program refilling 40 ms after
every wake (`a_program_slow_to_refill_has_the_low_water_s_time`, kept as a
test) runs dry without it. The mark stays (the emulator found it, ADR-0032
"S2 as built"); a test that bites without it (a program whose ring holds
no slack, or a burst of streams) is quality audit C18.

**Added 2026-10-19 night (43 mutants, all killed):** the load protocol
(frame size, CRC, file names, base64), the PE loader (section count, order,
CPU type), LZ4 (match offsets on both paths, the output bound, the fast
path's room), IPC (rights on grant and call, generations, nesting, the
handle quota), the FAT boot-file guard (MBR, boot sector, chains, chain ends,
data, renames: it had none), the memory manager (frame ranges, double frees,
small, section and large mappings, reserved domains) and USB mass storage
(range, transfer shape, write protection, command signature); then CompactFlash
(the range, a card held in reset), the PC tool (the emulator clean-up's port
match, the silent audio backend, the kit server's allow-list), FAT (FAT12/16
taken for FAT32, a trailing dot) and the cab decompressor (an over-subscribed
code, a literal past the limit). Three real test
gaps found and closed: nothing moved a boot file's chain end
(`a_boot_files_chain_end_cannot_be_moved`) and nothing mapped a large page
over small ones (`a_large_page_never_lands_on_small_pages`), and the cab
decompressor's limit was tested only through a match, never a literal
(`output_limit_is_enforced` now checks both and that nothing past the limit is
written). Two older emulator mutants were removed as equivalent (today's run-limit cap, now doubled by the 50 ms pen look; the CF attach on insertion, now doubled by the retry), and one new mutant was dropped as a layered guard (FAT's next-cluster
range, caught by the next step's own check), not a gap.

## `xtask/src` (19)

- `a_free_lock_is_free_even_with_a_stale_note` (xtask/src/devlock.rs)
- `a_press_answer_counts_only_near_the_pen` (xtask/src/emu.rs)
- `a_pushed_run_log_is_not_a_reply` (xtask/src/usb.rs)
- `bad_replies_are_refused` (xtask/src/trace.rs)
- `counts_only_allocated_sections` (xtask/src/size.rs)
- `hostile_or_unexpected_input_is_refused` (xtask/src/userimg.rs)
- `images_that_never_return_are_known` (xtask/src/size.rs)
- `mock_only_as_dev_dependency` (xtask/src/policy.rs)
- `only_bench_dir_creates_or_deletes` (xtask/src/usbbench.rs)
- `os_crates_never_depend_on_host_tools` (xtask/src/policy.rs)
- `refined_tables_never_fall` (xtask/src/calib.rs)
- `rejects_a_black_screen` (xtask/src/screen.rs)
- `rejects_an_empty_boot_log` (xtask/src/screen.rs)
- `rejects_non_elf` (xtask/src/size.rs)
- `rejects_truncated_ppm` (xtask/src/screen.rs)
- `rejects_wrong_size` (xtask/src/screen.rs)
- `the_frame_is_refused_for_its_crc_only` (xtask/src/usbbench.rs)
- `the_load_is_bounded` (xtask/src/emu.rs)
- `upward_edges_are_rejected` (xtask/src/policy.rs)

## `kernel/fat` (18)

- `a_cut_long_name_never_matches_its_prefix` (kernel/fat/src/exfat.rs)
- `a_damaged_entry_set_is_skipped_and_a_damaged_boot_region_refused` (kernel/fat/src/exfat.rs)
- `a_fat_chain_that_loops_is_corrupt_not_endless` (kernel/fat/src/tests.rs)
- `a_file_too_fragmented_to_record_gives_no_guard` (kernel/fat/src/guard_tests.rs)
- `a_folder_that_points_back_at_the_root_is_corrupt_not_a_loop` (kernel/fat/src/exfat.rs)
- `a_folder_that_points_back_at_the_root_is_corrupt_not_a_loop` (kernel/fat/src/tests.rs)
- `a_root_directory_that_loops_is_corrupt_not_endless` (kernel/fat/src/tests.rs)
- `checking_hostile_exfat_cards_never_panics` (kernel/fat/src/exfat.rs)
- `checking_hostile_volumes_never_panics_or_hangs` (kernel/fat/src/tests.rs)
- `exfat_power_cuts_leak_space_at_worst_never_damage` (kernel/fat/src/exfat.rs)
- `hostile_cards_never_panic_or_loop` (kernel/fat/src/tests.rs)
- `hostile_exfat_cards_never_panic_or_loop` (kernel/fat/src/exfat.rs)
- `out_of_range_clusters_and_absurd_geometry_are_refused` (kernel/fat/src/tests.rs)
- `power_cuts_leak_space_at_worst_never_damage` (kernel/fat/src/tests.rs)
- `replace_refuses_when_the_card_is_full_and_keeps_the_old_file` (kernel/fat/src/tests.rs)
- `sets_across_sector_boundaries` (kernel/fat/src/exfat.rs)
- `too_big_is_refused_with_the_card_unchanged` (kernel/fat/src/exfat.rs)
- `writing_to_hostile_exfat_cards_never_panics` (kernel/fat/src/exfat.rs)

## `kernel/mm` (13)

- `a_page_made_read_only_while_aged_comes_back_read_only` (kernel/mm/src/space.rs)
- `a_private_copy_is_compressed_not_dropped` (kernel/mm/src/space.rs)
- `fcse_relocates_only_the_bottom_slot` (kernel/mm/src/layout.rs)
- `half_a_large_page_cannot_be_unmapped` (kernel/mm/src/pagetable.rs)
- `large_unmaps_never_lose_pages` (kernel/mm/src/space.rs)
- `misaligned_tables_are_refused` (kernel/mm/src/arm.rs)
- `never_merges_into_frames_it_does_not_own` (kernel/mm/src/frames.rs)
- `only_anonymous_compressible_pages_are_compressed` (kernel/mm/src/space.rs)
- `protect_rewrites_attributes_and_invalidates` (kernel/mm/src/pagetable.rs)
- `take_stored_drains_in_bounded_batches` (kernel/mm/src/pagetable.rs)
- `the_quota_and_memory_bound_it` (kernel/mm/src/regions.rs)
- `unmap_cleans_while_mapped_then_clears_then_invalidates` (kernel/mm/src/pagetable.rs)
- `xscale_bit_4_is_never_set` (kernel/mm/src/arm.rs)

## `hal/pxa27x` (9)

- `arming_a_pin_clears_only_its_own_stale_edge` (hal/pxa27x/src/soc.rs)
- `cs3_timing_changes_only_the_strobes` (hal/pxa27x/src/diag.rs)
- `enable_and_disable_touch_only_their_bit` (hal/pxa27x/src/soc.rs)
- `gpio_edges_arm_and_disarm_only_their_pin` (hal/pxa27x/src/soc.rs)
- `invalid_shapes_are_refused` (hal/pxa27x/src/dmachain.rs)
- `mdrefr_slows_refresh_only_after_and_halves_before` (hal/pxa27x/src/clocks.rs)
- `refuses_illegal_points` (hal/pxa27x/src/clocks.rs)
- `rejects_out_of_range` (hal/pxa27x/src/lib.rs)
- `rejects_unaligned_regions` (hal/pxa27x/src/mmu.rs)

## `tools/romdump` (9)

- `out_of_bounds_is_an_error` (tools/romdump/src/bytes.rs)
- `rejects_bad_block_signature` (tools/romdump/src/imgfs.rs)
- `rejects_implausible_header` (tools/romdump/src/pe.rs)
- `rejects_missing_signature` (tools/romdump/src/nb0.rs)
- `rejects_non_xip` (tools/romdump/src/xip.rs)
- `rejects_partition_past_end` (tools/romdump/src/nb0.rs)
- `rejects_section_count_mismatch` (tools/romdump/src/pe.rs)
- `rejects_wrong_guid` (tools/romdump/src/imgfs.rs)
- `relocation_directory_larger_than_stored_data_is_clamped` (tools/romdump/src/pe.rs)

## `personality/ce` (8)

- `a_short_or_foreign_file_is_not_a_cabinet` (personality/ce/cab/src/lib.rs)
- `every_truncation_is_an_error_not_a_panic` (personality/ce/cab/tests/fixtures.rs)
- `hostile_mutations_never_panic_or_hang` (personality/ce/cab/tests/fixtures.rs)
- `hostile_scripts_never_panic` (personality/ce/setup/tests/scripts.rs)
- `hostile_setup_xml_never_panics` (personality/ce/setup/tests/scripts.rs)
- `paths_refuse_escapes_and_odd_characters` (personality/ce/setup/src/lib.rs)
- `random_bytes_behind_a_valid_signature_never_panic` (personality/ce/cab/tests/fixtures.rs)
- `spanning_sets_are_refused_by_flag_and_by_file_entry` (personality/ce/cab/tests/fixtures.rs)

## `drivers/mbx-display` (7)

- `ack_writes_back_only_the_rest` (drivers/mbx-display/src/lib.rs)
- `compose_applies_gamma_to_the_graphics_plane_only_while_enabled` (drivers/mbx-display/src/scanout.rs)
- `decode_refuses_what_it_cannot_draw_into` (drivers/mbx-display/src/lib.rs)
- `disable_clears_only_sync_enable` (drivers/mbx-display/src/lib.rs)
- `fill_rect_touches_only_the_rectangle` (drivers/mbx-display/src/lib.rs)
- `rejects_unaligned_plane` (drivers/mbx-display/src/lib.rs)
- `rejects_unaligned_video` (drivers/mbx-display/src/layers.rs)

## `drivers/mbx3d-render` (7)

- `address_modes_wrap_mirror_and_clamp` (drivers/mbx3d-render/src/tests.rs)
- `every_depth_and_alpha_mode_is_accepted_and_only_those` (drivers/mbx3d-render/src/tests.rs)
- `lists_are_reclaimed_between_frames_and_too_much_is_refused` (drivers/mbx3d-render/src/tests.rs)
- `render_done_raises_the_interrupt_only_when_switched_on` (drivers/mbx3d-render/src/tests.rs)
- `slots_draw_in_order_and_out_of_range_is_refused` (drivers/mbx3d-render/src/tests.rs)
- `targets_that_dont_fit_are_refused` (drivers/mbx3d-render/src/tests.rs)
- `writes_only_changes` (drivers/mbx3d-render/src/shadow.rs)

## `drivers/pxa-udc` (7)

- `a_pc_that_talks_or_a_configured_device_is_never_unplugged` (drivers/pxa-udc/src/unplug.rs)
- `a_rejected_table_is_reported_and_the_udc_left_off` (drivers/pxa-udc/src/tests.rs)
- `random_setup_packets_never_panic_or_overrun` (drivers/pxa-udc/src/tests.rs)
- `the_host_controller_never_runs_without_our_communication_area` (drivers/pxa-udc/src/unplug.rs)
- `the_serial_only_fallback_is_the_old_layout` (drivers/pxa-udc/src/tests.rs)
- `the_transceiver_output_is_turned_on_only_when_off` (drivers/pxa-udc/src/tests.rs)
- `udccr_writes_carry_only_writable_bits` (drivers/pxa-udc/src/tests.rs)

## `drivers/cf` (6)

- `a_bad_sector_is_an_error_with_the_card_usable_after` (drivers/cf/src/tests.rs)
- `a_card_that_never_gets_ready_times_out` (drivers/cf/src/tests.rs)
- `a_dma_that_misreads_refuses_or_fails_is_handled` (drivers/cf/src/tests.rs)
- `a_window_that_moves_too_little_is_refused_and_leaves_nothing_behind` (drivers/cf/src/tests.rs)
- `a_window_that_reads_wrong_keeps_the_port` (drivers/cf/src/tests.rs)
- `hostile_cards_never_panic_or_hang` (drivers/cf/src/tests.rs)

## `services/audio` (6)

- `a_dropped_switch_on_is_reported_and_undone` (services/audio/src/tests.rs)
- `a_late_wake_plays_silence_never_an_old_lap_and_nothing_is_lost` (services/audio/src/mixer/tests.rs)
- `full_scale_never_overflows` (services/audio/src/resample.rs)
- `going_down_from_sleep_stops_the_link_and_arms_the_guard` (services/audio/src/tests.rs)
- `the_guard_covers_the_measured_window` (services/audio/src/tests.rs)
- `waking_from_sleep_needs_no_guard_and_no_link_restart` (services/audio/src/tests.rs)

## `drivers/usb-msc` (5)

- `a_changed_medium_comes_back_only_once_the_host_noticed_or_after_the_ceiling` (drivers/usb-msc/src/tests.rs)
- `a_refused_block_is_reported_as_write_protected` (drivers/usb-msc/src/tests.rs)
- `random_commands_never_panic_or_reach_past_the_card` (drivers/usb-msc/src/tests.rs)
- `refused_or_failing_dma_falls_back_or_fails_cleanly` (drivers/usb-msc/src/tests.rs)
- `unknown_commands_and_bad_luns_fail_with_sense` (drivers/usb-msc/src/tests.rs)

## `drivers/gfx` (4)

- `hostile_frames_draw_nothing_wrong` (drivers/gfx/src/render/tests.rs)
- `layers_refuse_buffers_too_small_or_misaligned` (drivers/gfx/src/layer/tests.rs)
- `rectangles_clip_and_never_overflow` (drivers/gfx/src/tests.rs)
- `surfaces_refuse_buffers_too_small` (drivers/gfx/src/tests.rs)

## `drivers/pxa-mmc` (4)

- `a_card_that_never_finishes_programming_is_a_data_error` (drivers/pxa-mmc/src/lib.rs)
- `commands_without_data_never_set_the_bus_width` (drivers/pxa-mmc/src/lib.rs)
- `refuses_impossibly_slow_clocks` (drivers/pxa-mmc/src/lib.rs)
- `single_blocks_and_refused_buffers_use_the_fifo` (drivers/pxa-mmc/src/lib.rs)

## `pc/legacysync` (4)

- `a_script_naming_a_file_the_cab_lacks_is_refused` (pc/legacysync/tests/stage.rs)
- `cab_names_escaping_the_ini_folder_are_refused` (pc/legacysync/tests/stage.rs)
- `other_processors_and_refused_platforms_are_unusable` (pc/legacysync/src/select.rs)
- `the_check_and_the_reinstall_guard` (pc/legacysync/src/registration.rs)

## `drivers/pxa-gpio` (3)

- `make_input_clears_only_its_direction_bit` (drivers/pxa-gpio/src/lib.rs)
- `rejects_missing_pins` (drivers/pxa-gpio/src/lib.rs)
- `watching_edges_changes_only_its_pin` (drivers/pxa-gpio/src/lib.rs)

## `drivers/pxa-uart` (3)

- `a_dead_transmitter_never_hangs` (drivers/pxa-uart/src/lib.rs)
- `divisor_rejects_impossible_rates` (drivers/pxa-uart/src/lib.rs)
- `read_byte_takes_only_waiting_bytes` (drivers/pxa-uart/src/lib.rs)

## `drivers/sdcard` (3)

- `a_block_too_slow_even_alone_is_read_at_a_slower_clock` (drivers/sdcard/src/lib.rs)
- `a_card_that_never_finishes_programming_times_out` (drivers/sdcard/src/lib.rs)
- `bad_cmd8_echo_is_rejected` (drivers/sdcard/src/lib.rs)

## `kernel/ipc` (3)

- `a_table_grows_into_pages_only_when_it_needs_them` (kernel/ipc/src/tests.rs)
- `out_of_memory_refuses_the_handle_and_changes_nothing` (kernel/ipc/src/tests.rs)
- `random_operations_never_break_the_rules` (kernel/ipc/src/tests.rs)

## `shell/core` (3)

- `a_new_minute_redraws_only_the_clocks` (shell/core/src/tests.rs)
- `a_tap_outside_the_menu_closes_it_and_does_nothing_else` (shell/core/src/tests.rs)
- `back_to_loader_only_in_test_images` (shell/core/src/tests.rs)

## `boot/mved` (2)

- `a_conversion_must_finish_inside_its_guards` (boot/mved/src/main.rs)
- `only_a_pass_goes_on` (boot/mved/src/main.rs)

## `boot/noise` (2)

- `a_row_is_appended_whole_and_a_full_file_refused` (boot/noise/src/main.rs)
- `only_healthy_rows_go_into_the_restart_test` (boot/noise/src/main.rs)

## `drivers/mved` (2)

- `jobs_aimed_outside_their_frames_are_refused` (drivers/mved/src/tests.rs)
- `only_dells_three_registers_are_readable` (drivers/mved/src/tests.rs)

## `drivers/touch-cal` (2)

- `a_bounce_while_landing_is_not_a_lift` (drivers/touch-cal/src/tests.rs)
- `the_most_points_fit_without_overflow` (drivers/touch-cal/src/tests.rs)

## `hal/mock` (2)

- `rejects_odd_16_bit_offsets` (hal/mock/src/lib.rs)
- `rejects_unaligned_offsets` (hal/mock/src/lib.rs)

## `kernel/power` (2)

- `idle_with_the_screen_on_never_ramps` (kernel/power/src/lib.rs)
- `unmeasured_costs_never_go_deeper` (kernel/power/src/lib.rs)

## `kernel/trace` (2)

- `garbled_dumps_are_refused_or_bounded` (kernel/trace/src/lib.rs)
- `torn_and_garbled_records_are_dropped_not_misread` (kernel/trace/src/lib.rs)

## `services/graphics` (2)

- `clients_reach_only_their_own_memory_within_their_quota` (services/graphics/src/tests.rs)
- `without_3d_state_frames_are_2d_and_3d_is_refused` (services/graphics/src/tests.rs)

## `boot/loadproto` (1)

- `random_streams_never_panic` (boot/loadproto/src/lib.rs)

## `boot/looks` (1)

- `damaged_photos_are_refused` (boot/looks/src/lib.rs)

## `boot/mbx-blend` (1)

- `sweep_is_bounded_and_starts_with_dell` (boot/mbx-blend/src/main.rs)

## `boot/probe` (1)

- `full_pages_count_overflow` (boot/probe/src/report.rs)

## `drivers/draw` (1)

- `text_is_stored_and_bounded` (drivers/draw/src/tests.rs)

## `drivers/mbx3d` (1)

- `region_array_refuses_what_wont_fit` (drivers/mbx3d/src/tests.rs)

## `drivers/pxa-i2s` (1)

- `frames_go_only_when_the_fifo_has_room` (drivers/pxa-i2s/src/lib.rs)

## `drivers/pxa-keypad` (1)

- `a_tap_between_two_scans_is_counted_never_silent` (drivers/pxa-keypad/src/keys.rs)

## `drivers/pxa-ssp` (1)

- `refuses_impossibly_slow_clocks` (drivers/pxa-ssp/src/lib.rs)

## `drivers/tsc2046` (1)

- `early_values_only_inside_the_settling` (drivers/tsc2046/src/lib.rs)

## `drivers/vram-map` (1)

- `double_frees_and_impossible_requests_are_refused` (drivers/vram-map/src/tests.rs)

## `drivers/wm8750` (1)

- `a_gated_codec_is_found_and_the_gate_never_stays_open` (drivers/wm8750/src/lib.rs)

## `hal/api` (1)

- `insert_refuses_values_that_do_not_fit` (hal/api/src/lib.rs)

## `hal/mem` (1)

- `volatile_word_access_checks_bounds_and_alignment` (hal/mem/src/lib.rs)

## `kernel/pe` (1)

- `values_straddling_a_page_boundary_are_fixed_from_both_sides` (kernel/pe/src/tests.rs)
