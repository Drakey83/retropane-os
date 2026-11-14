# Contacts, Rule 0

Batch 1 of [core-apps-list.md](core-apps-list.md). Look:
[design-language.md](design-language.md). Store: [pim-store.md](pim-store.md)
(the one made for Calendar; no second store). Method:
[principles.md](principles.md). Built without a stop for the owner's go;
the notes for his morning test are at the end.

## The core job

Find a person fast and read their details; add and change people. v1:
the card list, the letter strip, fast search, add and edit. Photos and
vCard import and export are on the later list.

## What others do (findings, not the answer)

| System | Finding someone | The card | Adding |
|---|---|---|---|
| Windows Mobile 5 Contacts | A list by "Last, First"; letter tabs in groups (`#ab`, `cde`...) that cycle on repeated taps; a "quick find" box matching name prefixes only | A summary page, then an edit form of some forty fields in one long scroll | The forty-field form, every field shown whether used or not |
| Palm OS Address | A list with the phone number beside the name; "Look Up" by last-name prefix | One page | A shorter form; five phone lines with a type picker each |
| iOS Contacts | The A-Z index strip down the right edge, dragged to scrub; a search bar that matches any part of any field | A page showing only the fields filled | A form that starts short and grows ("add phone") |
| Android Contacts | Search first; favourites on top | Only filled fields | As iOS |
| Lean Linux (`abook`) | Type to filter | A text card | A few fields |

## Borrow and beat

| Problem | Borrowed | How ours goes further on the X51v |
|---|---|---|
| Jumping in a long list | iOS's index strip: one letter per position, tap or drag | A 26-letter strip down the right edge, 17 px a letter (a stylus hits it; WM grouped letters because fingers couldn't). Letters with nobody under them are dimmed, so a tap is never wasted. Up and down on the D-pad move a row; left and right jump a letter |
| Search | iOS's any-field search | The field is always at the top of the list, and it matches **names, company, any phone's digits and e-mail, and initials** ("jd" finds John Doe; "555" finds a number). WM matched name prefixes only. The search runs in RAM over the store when a key is typed: under 2,048 records it is a few milliseconds, with no index to keep |
| Reading a card | iOS: only what is filled | The card shows only filled fields, each a row with its label in secondary text; nothing to scroll past |
| Adding | iOS's form that starts short | Nine rows that fit one screen above the keyboard without scrolling for the first five: First, Last, Company, Mobile, Home, Work, E-mail, Address, Notes. The keyboard's Done moves to the next row, so a card is typed top to bottom without lifting to tap each field |
| Sort order | WM's "Last, First" | By last name, then first; a card with no last name sorts by first name, one with neither by company. Shown as "First Last" with the sort letter in the strip, so the list reads as names, not as a filing system |

## The design (v1 as built)

- **The list:** header "CONTACTS"; the search field (40 px) under it; rows
  of 36 px: the name left, the first of mobile, home, work or company in
  secondary text right; the letter strip on the right edge. Softkeys
  **Back** and **New**. Empty: "No contacts yet. New adds the first one."
  A search with no match says "No one matches".
- **Search:** tapping the field brings the keyboard; the list filters at
  each key; the softkeys become Cancel (clears the search) and Done
  (keeps the filter, hides the keyboard).
- **The card:** the name as the header; a row per filled field (label
  left, value right; the address and notes wrap to two lines). Softkeys
  **Back** and **Edit**.
- **The editor:** the nine rows; tapping one types into it. Delete (for
  an existing card) asks once, in place (the row then reads "Tap again
  to delete"). Softkeys Cancel and
  Done; a card with every field empty isn't kept.

## How the design was iterated

1. *Draft:* WM's grouped letter tabs across the top. **Lost:** two taps
   to reach most letters, and a row of height the list needs. The strip
   on the edge costs 18 px of width and one tap.
2. *Draft:* search behind a softkey. **Lost:** search is the faster way
   past about fifty people; hiding it makes the strip the only way. The
   field stays on screen.
3. *Draft:* the phone number shown under the name (two-line rows).
   **Lost:** it halves the people on screen (13 to 8). One line, the
   number right-aligned.
4. *Draft:* separate "File as" and display name (WM, Outlook). **Lost:**
   a setting to explain for no gain in v1; one rule (last, first,
   company) sorts every card sensibly.
5. *Draft:* a field per phone type with a picker (Palm). **Lost:** three
   fixed phone rows cover what people carry, with no picker.

## The practices (principles.md)

- **P1:** the sorted list and the strip's letters are rebuilt when the
  store changes or the search text does, never while the pen moves.
- **P6:** the list is 1,024 two-byte entries in the shell's own memory;
  no frame over 4 KB.
- **Frame rule:** a screen of 13 rows, the strip and the header is about
  110 draw commands.

## Later (not in v1)

Photos; vCard import and export; birthdays and anniversaries (with the
Calendar tie-in); more than three phones and custom labels; categories
and groups; favourites; dialing and messaging hooks (no phone in the
X51v; Bluetooth later); beaming a card; duplicate merging; names in
scripts beyond ASCII (the font's limit today: the store keeps UTF-8, the
fonts decide what shows); sorting by first name as a setting.

## Notes for the owner's morning test

- **Sorted by last name, shown "First Last".** Alternative: shown "Last,
  First" as WM did.
- **Search matches initials and phone digits** as well as text.
- **Three phone rows** (Mobile, Home, Work), no type picker.
- **The keyboard's Done goes to the next field**; the softkey Done keeps
  the card.

## What would overturn it

- The 17 px strip letters being too small for his stylus on the panel:
  then 13 groups of two letters at 34 px.
- More than about 1,500 contacts: the store's paging design.
