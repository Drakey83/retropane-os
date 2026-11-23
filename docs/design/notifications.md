# Notifications: the reminder service (version 1, 2026-11-23)

One service reminds for Calendar and Tasks: a reminder comes up at its
time whether its app is open or not (the apps are their own programs,
loaded when opened and released when left: `shell/today/src/program.rs`),
shows a banner over any screen, sounds a chime, and offers snooze and
open. It answers the build-list rows of [calendar-rule0.md](calendar-rule0.md)
("Reminders as a banner over any screen, several per appointment",
"Reminder sound and snooze") and [tasks-rule0.md](tasks-rule0.md)
("Reminders through the notification banner and sound"). The wider
notification system the owner asked for on 2026-11-15 (a centre of what
came, sounds to choose, per-app switches, quiet hours, a Settings
program: R243) builds on this one.

## What others do (findings, not the answer)

| System | How a reminder is kept and fired | What it costs |
|---|---|---|
| Windows Mobile 5 | Each app registers its alarm with the kernel's notification database (`CeSetUserNotificationEx`); the RTC alarm wakes the device; a modal bubble with Dismiss and a Snooze drop-down; `Alarm1.wav` through the mixer | A second copy of every time, stale when a sync changes the record; one alert per appointment; the bubble takes the keys |
| iOS 18 | Apps schedule `UNNotificationRequest`s with the OS (64 pending per app); two alerts per event; a banner from the top, pulled down for actions | The app must re-schedule after every change; the cap |
| Android 14 | `AlarmManager.setExactAndAllowWhileIdle`, re-registered after each reboot; heads-up banner with actions; a channel per kind with its own sound | Exact alarms need a permission; the app is woken to post |
| Google Calendar | Five alerts per event | Needs the phone's alarm service |
| Lean Linux (calcurse, remind) | A daemon re-reads the files each minute | A wake a minute, all day |

## Ours, and why it wins

- **No second copy.** The alerts are read from the one store
  ([pim-store.md](pim-store.md)) by [`rp_pim::remind`](../../shell/pim/src/remind.rs):
  nothing registered, nothing to forget or re-register after a reboot, and
  a record changed by the app, a sync or an import reminds at its new time.
  An app program that changes the records has the banner follow at once
  (`Shell::reminders_follow`): a task ticked done or an appointment moved
  or deleted takes its card away, never leaving a card for nothing.
- **Tickless.** The next alert is worked out once per change of the
  records (each record's leads against the days near the window, a
  repeating appointment tested with `Rule::occurs`, no occurrences
  expanded); a minute's tick is one comparison. `Shell::next_reminder` is
  the one minute the device must be awake for: when the OS sleeps (the
  power service, not built: R369) that minute goes into the PXA270's RTC
  alarm and nothing else wakes it.
- **Several alerts.** Up to five leads per appointment (Google's five;
  [`tag::REMIND`](../../shell/pim/src/store.rs) holds one little-endian
  `u32` each, the old single value still reads), set as chips that turn on
  and off in Calendar's editor (`apps/calendar/src/lib.rs`, the ALERTS
  row: NONE, START, 15 MIN, 1 HOUR, 1 DAY and one of any length). An
  imported appointment's leads the chips can't show are kept unless the
  row is changed. A later alert of the same appointment takes the earlier
  one's place on the banner: one card per appointment.
- **Every occurrence.** A repeating appointment reminds before each of
  its days, with that day's time on the card (before, only its first
  occurrence ever reminded).
- **A task** reminds at its time while it is not done (R223); OPEN goes to
  Tasks.
- **Snooze 5 and 15 minutes** (calendar-rule0.md), each a key on the
  banner; the snoozed alert comes back with its sound.
- **Never steals the keys.** The banner answers the stylus only; the
  D-pad and the softkeys stay the app's (WM's bubble took them). Over an
  open app its keys are the banner's (they were dead there: the app's
  area took the touch first); a stroke that starts on the app stays the
  app's.
- **The sound** ([`rp_audio::chime`](../../services/audio/src/chime.rs)):
  two notes a fourth apart (1568 and 2093 Hz) with a 4 ms rise, a small
  bell's decay and a 6 ms fade, the second struck while the first rings:
  no edge to click (the first chime, R224, switched two sines on and off),
  no wave file (WM's is tens of KB), made as it plays. The notes are where
  the X51v's speaker is loudest (docs/hw/audio.md, its sweep: flat 1.5-2
  kHz, about 20 dB an octave down below 1 kHz), so 9 and 4 dB louder than
  880 and 1175 Hz were at the same level. Played through the audio service
  (ADR-0032), the codec powered only while it sounds.

## The banner's look: A, the owner's pick (2026-11-23)

Three looks were drawn for his pick; he chose A, the card under the
title bar, and B and C were taken out (R370). Drawn by the shell's own
code (`shell/core/src/apps/mod.rs`, `LAYOUT`;
`RP_PREVIEW_ONLY=notify cargo xtask ui-preview`, in
`target/ui-preview/notify-banner*.png`), in the Aero glass: the wallpaper's
frosted panel, the apps' gel keys, OPEN the blue glow, the record's
calendar or list colour as a bar down its words (with the app's name:
colour never alone).

| Look | What | For | Against |
|---|---|---|---|
| **A. Card** (picked, built) | A card under the title bar: CALENDAR IN 15 MIN, the title, the time and place; a row of four keys 5 MIN, 15 MIN, DISMISS, OPEN | Every word and key large; four equal 106 x 34 keys | Covers the app's top 124 px (its large title) |
| B. Strip (not picked, taken out) | 84 px: the words left, 5 MIN, 15 MIN and OPEN right, an X to dismiss | Covers least | Title cut sooner (about 210 px of it); a small X |
| C. Bottom (not picked, taken out) | Card A over the bottom of the app, above the softkeys | Near the hand; the app's title stays in view | Covers the foot tabs, and the keyboard when it is up |

## Build list

| Item | Status | Code |
|---|---|---|
| Alerts read from the store, several per appointment, every occurrence of a repeat; next alert per change; next wake | built | shell/pim/src/remind.rs, shell/core/src/apps/mod.rs |
| Banner over any screen and over an open app; follows the records; pressed keys | built | shell/core/src/apps/mod.rs, shell/core/src/lib.rs, shell/core/src/tests.rs |
| Snooze 5 and 15 minutes | built | shell/core/src/apps/mod.rs |
| Several alerts in Calendar's editor | built | apps/calendar/src/lib.rs, apps/calendar/src/tests.rs |
| The chime | built | services/audio/src/chime.rs, shell/today/src/main.rs |
| The banner's look: A, the owner's pick (R370) | built | shell/core/src/apps/mod.rs, xtask/src/app_mockups.rs |
| The RTC alarm wake when the OS sleeps | waiting: the OS's sleep (R369) | |

## What would overturn it

- The owner hearing the chime too quiet or harsh on the X51v (R224's run):
  the level, the notes or the decay change; the speaker's measured band
  stays the guide.
- Alerts missed after a long time off: the late window (an alert within
  the hour is shown when the records change; on a start, a day) is set
  again from what he sees.
