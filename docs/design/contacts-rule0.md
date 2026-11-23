# Contacts, Rule 0 (version 2, 2026-11-14)

Version 1 (2026-11-13) built a list of grey rows and a plain form: phone
numbers not formatted, no capitals in names or addresses, no address
help, no photos, everything modern under "Later". The owner rejected it
on 2026-11-14 ("these are all modern normalcies"). This version builds
what the survey found, under the rules gate's rule 6
([app-scope.md](app-scope.md)). Look: [design-language.md](design-language.md)
version 2 (its field-aware keyboard is where phone formatting, capitals
and address suggestions live, shared by every app). Records: the one store
in **internal storage**. Plans it answers to: [apps-and-packs.md](apps-and-packs.md)
(the Contacts row; §7's sync machinery, now for contacts too),
[ui.md](ui.md) (SPB Mobile Shell's favourite contacts as large photos),
[interaction.md](interaction.md) I10 (the Contacts button opens Contacts).

## The core job

Find a person in a second, see how to reach them, and add or fix a card
without fighting the keyboard; the same people as the phone and the PC.

## What others do (findings, not the answer)

| System | What it does well | What it costs |
|---|---|---|
| Windows Mobile 5 | A card list with a letter strip; Outlook sync over ActiveSync | Plain "Last, First" rows; a long form of small fields; numbers kept as typed; no photos in the list |
| iOS Contacts 18 | A contact poster (large photo or initials on colour); an action row (message, call, mail); grouped fields; numbers formatted as typed; capitals by field | One long list; favourites live in Phone |
| Google Contacts (Android) | Coloured initials for everyone without a photo; search across every field; duplicates found and merged; address fields with Maps suggestions; CardDAV-like sync of any Google account | Needs Google for the suggestions |
| SPB Mobile Shell (WM) | Favourite contacts as a 3-across grid of large photos; an action sheet of big tiles | Photos needed for it to look right |
| Lean Linux (khard + vdirsyncer) | Plain vCards on disk, synced by CardDAV | Text only |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Finding someone | Google's search over every field; WM's letter strip | Search matches **initials, any part of a name, company, phone digits ignoring formatting, e-mail and city**, with the match highlighted; the letter strip shows a **large letter bubble** under the pen; the list has letter headers |
| Seeing who it is | iOS's initials on colour; SPB's photo grid | **A favourites grid** of large photos or coloured initials (3 across) at the top of the list; every row has a 36 px avatar; a person's colour is a hash of their name (never changes) |
| The card | iOS's poster and action row | A card with the photo or initials large, the name in 28 px, the company under it, then an **action row** (E-mail, Map, Copy, Send to PC) and grouped details; tapping a number or address copies it with a toast |
| Typing a number | iOS's formatting | A **phone pad** opens for phone fields; the number **formats as typed** for its country ((206) 555-0142; +44 20 7946 0958) from a compact per-region table; stored as digits plus country (E.164), shown in the region's format |
| Typing a name or address | phones' capitals by field | **Capitals at each word** in names, companies and addresses; e-mail fields get `@` and `.com` keys and no capitals |
| Typing an address | Google's Maps suggestions | **Suggestions as you type** from Photon (OpenStreetMap) over any connection, after a 300 ms pause, five answers; a tap fills street, city, state, ZIP and country; offline, a US ZIP fills city and state from a table in internal storage, and past addresses are offered |
| Where they are | Google's map link | **Map** opens the address on the radar's own map tiles (the Weather app's map work, ui.md), with a pin |
| Many fields | iOS's "add field" | Fixed groups for the common (phones, e-mails, address) with **+ add** for more of each, custom labels, birthday, anniversary, website, notes |
| Duplicates | Google's merge | Cards with the same phone or e-mail are offered as **one merge** at import or sync |
| Same people everywhere | WM's Outlook sync | **vCard 3/4 import and export** (one file or many, from the PC or the card) and **CardDAV** (iCloud, Fastmail, Nextcloud) and **Google** through the same Connections page and sync scheduler as Calendar, over any connection |
| Birthdays | phones' contact birthdays | A contact's birthday shows in Calendar's Birthdays calendar |
| The PDA's own touches | WM's Contacts button | The **Contacts button** opens Contacts (I10) |

## The design

- **Home:** large title "Contacts / 128 people", search field, the
  favourites grid (two rows of three, scrolling away with the list), then
  the list: letter header, rows of avatar, "First Last" in `ui`, company or
  the first number in secondary text. Letter strip on the right with its
  bubble. Softkeys **Back** and **New**.
- **Card:** avatar 96 px on a tinted band, name, company and title, the
  action row (four 64 px tiles), then cards: phones (label and formatted
  number), e-mails, address (multi-line, Map), birthday, notes. Softkeys
  **Back** and **Edit**.
- **Editor:** photo (choose from a picture file, cropped to a circle and
  kept small in internal storage), name fields (First, Last, Company,
  Title), then groups with + add: Phone (label picker: Mobile, Home, Work,
  custom), E-mail, Address (Street, City, State, ZIP, Country, with
  suggestions), Birthday, Website, Notes, Favourite switch. Delete asks in
  place. Softkeys **Cancel** and **Done**.

## Build list

| Item | From | Status | Code |
|---|---|---|---|
| Large title with the count; list with letter headers, avatars and secondary lines | sheet | built | apps/contacts/src/lib.rs |
| Favourites grid of large photos or initials | apps-and-packs: favourites | built | apps/contacts/src/lib.rs |
| Coloured initials for everyone without a photo | apps-and-packs: initials when there's no photo | built | apps/api/src/kit.rs, apps/contacts/src/lib.rs |
| Photos: chosen from the avatar (the card's or the editor's) out of the pictures in the photos folder on the PC, over the dock; cropped to a circle and kept small (about 6.9 KB a person: the disc only, brightness at five bits, colour at a quarter of the resolution); shown on the card, the list and the favourites; removed by a key, and with the person | apps-and-packs: photos | built | apps/contacts/src/photo.rs, apps/contacts/src/lib.rs, xtask/src/usb.rs |
| Photos from a picture file on the card, and from vCard PHOTO | apps-and-packs: photos | built | shell/pim/src/jpeg.rs, apps/contacts/src/io.rs, apps/contacts/src/photo.rs, shell/today/src/cardfiles.rs, apps/contacts/src/tests.rs |
| Search over every field with the match highlighted | apps-and-packs: Fast search | built | apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| Letter strip with the large letter bubble | sheet | built | apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| The card: the poster, the action row's Copy, and every line (three numbers, the e-mail, the address) copied by a tap or the D-pad with a COPIED mark | sheet | built | apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| The card's E-mail and Send to PC keys (drawn dim until then, R210; Map is its own row below) | sheet | blocked: owner 2026-11-15 sign off without them: no mail program and no PC side for a sent card exist yet; the keys are drawn dim (R210) | |
| The editor: every field typed (first, last, company, three numbers, e-mail, street, city), a favourite switch, a kept person changed or deleted (asked twice) | sheet | built | apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| The list in pages: a drag or the D-pad turns the page (a letter with more people than fit was cut off) | sheet | built | apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| Phone pad for phone fields; numbers formatted as typed for their country, stored as E.164 | apps-and-packs: phone numbers formatted as typed | built | shell/pim/src/phone.rs, shell/core/src/keyboard.rs |
| Address suggestions while typing (Photon over any connection; ZIP table and past addresses offline) | apps-and-packs: address suggestions while typing | built | shell/pim/src/net.rs, apps/contacts/src/lib.rs, xtask/src/usb.rs |
| Capitals at each word in names, companies and addresses; e-mail keyboard | apps-and-packs: capitals at each word in names and addresses | built | shell/core/src/lib.rs |
| Structured address, + add for more phones and e-mails, custom labels, birthday, anniversary, website | sheet | built | shell/pim/src/contact.rs, apps/contacts/src/form.rs, apps/contacts/src/lib.rs, apps/contacts/src/tests.rs |
| Map: the address on the radar's map tiles with a pin | sheet | pending | |
| Duplicates offered as one merge | sheet | built | shell/pim/src/contact.rs, shell/pim/src/vcard.rs, apps/contacts/src/io.rs, apps/contacts/src/tests.rs |
| vCard 3 and 4 import and export | apps-and-packs: vCard import/export | built | shell/pim/src/vcard.rs, apps/contacts/src/io.rs, shell/today/src/cardfiles.rs, shell/today/src/main.rs, xtask/src/usb.rs |
| CardDAV and Google contacts sync, read and write, over any connection | apps-and-packs: online contacts (CardDAV, Google) | blocked: owner 2026-11-15 sign off without it: CalDAV, CardDAV and Google sign-in need an app registration and accounts only the owner can create, and a server to test against | |
| Birthdays feed Calendar's Birthdays calendar | sheet | pending | |
| The Contacts button opens Contacts | sheet | built | shell/core/src/lib.rs, shell/core/src/tests.rs |
| The light look: the app's own light palette (its content is drawn after the shell's look pass, so rounded shapes keep one colour edge to edge) | sheet | built | shell/core/src/lib.rs, apps/api/src/kit.rs, tools/shell-assets/accents.py |
| Chips and cards as rounded shapes of one colour, no glass art inside (the owner, 2026-11-14: "a box on top of the button") | owner 2026-11-14 | built | apps/api/src/kit.rs, shell/core/src/panel_art.rs |
| Its own program, loaded when opened and released when left | owner 2026-11-14 | built | apps/contacts/src/main.rs, shell/today/src/program.rs |
| Records in internal storage (the card only as the test stage's stand-in) | owner 2026-11-14 | blocked: owner 2026-11-15 sign off without it: internal storage waits on the flash driver; the card stands in, same code and format | |

### Version 3 (2026-11-23): records and files, as built

- **The fields:** the job title under the name; Mobile, Home and Work
  kept where every older program reads them, and **+ Add a phone / + Add
  an e-mail** under each kind for more, each with a label of its own (a
  tap on the label types it, or the strip over the keyboard offers
  Mobile, Main, Other / Home, Work, Other); the address in its five
  parts (Photon's city line split into city, state and ZIP); birthday and
  anniversary typed as digits the region's way (12/10 or 12/10/1815 in
  the US, the day first elsewhere), a day that isn't one said on its row
  ("NOT A DAY") and nothing kept until it is; a website. The store keeps
  the extras packed in one field (`tag::MORE`), so a record never runs
  out of its 16 fields; the birthday is one number whose year 0004 means
  "no year" (`rp_pim::contact::Day`), what Calendar's Birthdays reads.
- **The card:** a line per number, e-mail (with their own labels), the
  address on two lines, the birthday with the age at the next one
  ("turns 211 next"), the anniversary, the website and the note's first
  line; more than fit go on over the page by the D-pad or a drag, with
  "1-6 of 11" on the band.
- **Import and export** (the key with three dots beside the search):
  the .vcf files on the card (its root and RetroPane folder) and the PC
  (Documents\RetroPane, Downloads, the Desktop) listed together; vCard
  2.1, 3.0 and 4.0 read with every writer's labels (Apple's groups and
  1604 years, Outlook's 2.1 quoted-printable and Latin-1); what the device
  doesn't keep counted and said; export of everyone as 3.0 (Outlook
  reads no 4.0) or 4.0, to the card or the PC, written a piece at a time.
- **Pictures:** from a vCard's PHOTO and from a JPEG on the card (the
  chooser lists the card's pictures beside the PC's), read with
  `rp_pim::jpeg`: a camera's own Exif thumbnail when it covers the
  square (a 12-megapixel photo without reading its 4 MB), else decoded
  straight to the size needed from the DCT's lowest frequencies,
  box-filtered and turned upright, in pieces; checked against Pillow on
  six kinds of file.
- **Duplicates:** after an import, and from the tools page, people with
  a number or an e-mail in common are offered one pair at a time: the
  two side by side, what they share, what the one card keeps ("As one: 3
  numbers, 2 e-mails, address"); **Merge into one** keeps every field of
  both (the picture the kept one lacks moved over), **Keep both** is
  remembered and never asked again.

## What would overturn it

- The favourites grid pushing the list too far down for him: then it
  folds to one row of five small avatars.
- Photon refusing as-you-type use from the device's volume: then the PC
  relay keeps a local Photon index of the user's region, or a keyed
  service the owner picks.
