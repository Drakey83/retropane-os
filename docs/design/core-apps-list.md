# The core apps: the checklist

The owner's direction of 2026-10-16
([apps-and-packs.md](apps-and-packs.md) §1): every program a stock Windows
Mobile 5 Pocket PC shipped, rebuilt for RetroPane. That page's table is
the starting point; this page completes it against what a stock WM5 Pocket
PC actually shipped, and puts it in build order, **five at a time**.

**The list is scope only.** Windows Mobile tells us *what* to include,
never how it looks or works: each app gets its own Rule 0 sheet
(`docs/design/<app>-rule0.md`) before it is built, and follows
[design-language.md](design-language.md).

A batch's five lines are ticked in the same commit as that batch's final
report, so this page is always the true record of what is done and what is
left.

**How it was checked (2026-11-13):** against the WM5 Pocket PC's Start
menu, its Programs folder (including Games), its Settings tabs (Personal,
System, Connections), and the X51v's own additions from Dell. Lines marked
*(added)* were not in the Oct 16 table.

## Batch 1: the organiser (report: [core-apps-2026-11-13.md](../reports/core-apps-2026-11-13.md))

- [x] **Calendar** (WM5: Calendar): appointments by day, agenda and month; reminders.
- [x] **Contacts** (WM5: Contacts): the people list with a letter strip and search.
- [x] **Tasks** (WM5: Tasks): a to-do list with due dates and priority.
- [x] **Notes** (WM5: Notes): short typed notes, searchable.
- [x] **Calculator** (WM5: Calculator): a basic calculator.

## Batch 2: time, files and finding things

- [ ] **Clock & Alarms** (WM5: Clock & Alarms): the time and date, home and visiting clocks, alarms.
- [ ] **File Explorer** (WM5: File Explorer): folders on the device and both cards; copy, move, rename, delete.
- [ ] **Search** (WM5: Search): find files and organiser entries by word.
- [ ] **Help** (WM5: Help): help pages for the system and each app.
- [ ] **Notification centre and Running Programs** *(added)* (WM5: the notification bubbles, and Memory > Running Programs): what needs attention, and what is running, in one place.

## Batch 3: Settings, the Personal tab

- [ ] **Settings shell** (WM5: Settings): one app holding every applet, grouped as WM's so people find things.
- [ ] **Today settings** (WM5: Today): the Today screen's theme, wallpaper and items.
- [ ] **Buttons and Input** *(added as its own line)* (WM5: Buttons; Input): what each hardware button does; the keyboard's options.
- [ ] **Owner Information and Lock** *(added)* (WM5: Owner Information; Lock, called Password before AKU2): who the device belongs to; a PIN or password.
- [ ] **Sounds & Notifications and Menus** *(added as its own line)* (WM5: Sounds & Notifications; Menus): event sounds and volumes; what the Start menu lists.

## Batch 4: Settings, the System tab

- [ ] **Brightness and Power** (WM5: Backlight or Brightness; Power): the screen's light and its schedule; the battery and power-off times.
- [ ] **Screen and Regional Settings** (WM5: Screen; Regional Settings): orientation, alignment, text size; units, dates and number formats.
- [ ] **Memory and Remove Programs** (WM5: Memory; Remove Programs): storage and RAM in use; uninstalling.
- [ ] **About and Certificates** (WM5: About; Certificates): the device's name and version; trusted certificates.
- [ ] **Error Reporting and GPS** *(added)* (WM5: Error Reporting; GPS): crash reports (ours stay on the device unless sent); the GPS port for add-on receivers.

## Batch 5: media and pictures

- [ ] **Pictures & Videos** (WM5: Pictures & Videos): the album grid, viewing, slideshows.
- [ ] **Media Player** (WM5: Windows Media Player 10 Mobile): music and video, playlists, the library.
- [ ] **Voice recorder** *(added)* (WM5: Notes' recording bar and the Record button): record and play voice notes.
- [ ] **Solitaire** (WM5: Solitaire): Klondike.
- [ ] **Bubble Breaker** (WM5: Bubble Breaker): the bubble puzzle.

## Batch 6: documents

- [ ] **Word Mobile** (WM5: Word Mobile): open and edit simple documents.
- [ ] **Excel Mobile** (WM5: Excel Mobile): open and edit simple spreadsheets.
- [ ] **PowerPoint Mobile** (WM5: PowerPoint Mobile): show presentations.
- [ ] **Ink notes** (WM5: Notes' handwriting): handwritten notes and drawings (Notes v1 is typed only).
- [ ] **Beam** *(added)* (WM5: Beam, the infrared send and receive): send a contact, note or file to a device nearby.

## Batch 7: the network (after networking lands, Phase 6)

- [ ] **Connections and Network Cards** *(added as its own line)* (WM5: Connections; Network Cards; the X51v's Dell WLAN Utility): Wi-Fi networks, addresses, proxies.
- [ ] **Bluetooth** *(added)* (WM5: Bluetooth): pairing and the device's Bluetooth services.
- [ ] **Browser** (WM5: Internet Explorer Mobile): RetroPane's own browser.
- [ ] **Messaging** (WM5: Messaging, the Inbox): mail accounts and folders.
- [ ] **Download Agent** *(added)* (WM5: Download Agent): downloads in progress and finished.

## Batch 8: the PC and the rest

- [ ] **LegacySync** (WM5: ActiveSync on the device): sync with the PC (ADR-0028).
- [ ] **Remote desktop** (WM5: Terminal Services Client): a remote Windows desktop.
- [ ] **Messenger and web mail** *(added)* (WM5: Pocket MSN: MSN Messenger and Hotmail): listed for completeness; the services are gone, so this becomes a decision for the owner (drop, or a modern chat client).
- [ ] **Data Backup** *(added)* (the X51v's Dell Data Backup): back up and restore the device's data to a card.
- [ ] **Switcher Bar** *(added)* (the X51v's Dell Switcher Bar): Dell's task switcher; folded into the task switcher unless the owner wants it as its own thing.

## Already built, outside the batches

- [x] **Today screen** (WM5: Today): the home screen (`shell/today`, Phase 5).
- [x] **Start menu** (WM5: Start): the menu in the corner (`shell/core`); the task switcher is in batch 2's line.
- [x] **Weather** (not in WM5; the owner's default home): `shell/weather`.
