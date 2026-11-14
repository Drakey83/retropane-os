# App scope: Rule 0 reaches the build

The owner, 2026-11-14, after core apps batch 1: each app's Rule 0 sheet
surveyed modern apps, then the build stopped at a 2005 baseline and filed
every modern feature, even the ones apps-and-packs.md already planned,
under "Later". This page is the registry the rules gate reads
(`tools/remind/rules_gate.py`, rule 6) so that can't happen again:

- A commit touching an app's code (the prefixes below) needs that app's
  sheet to hold a `## Build list` table: `| Item | From | Status | Code |`.
- **From** names where the item comes from: `apps-and-packs: <the phrase
  in its "Modernize" column>`, `owner YYYY-MM-DD`, or `sheet` (the sheet's
  own borrow-and-beat). Every phrase of the app's row in
  [apps-and-packs.md](apps-and-packs.md) must appear as a From.
- **Status** is `built` (Code names files that exist), `pending`, or
  `blocked: hardware ...`, `blocked: arch N ...` or `blocked: owner
  YYYY-MM-DD ...` with the reason (20 characters or more). No "Later"
  heading: what waits names its blocker.
- `cargo xtask ci` runs the same check strictly (`cargo xtask scope`): a
  `pending` row fails it, so no push carries an app whose plan isn't built.

## Registry

| App | Sheet | Code |
|---|---|---|
| Calendar | docs/design/calendar-rule0.md | shell/core/src/apps/calendar.rs, apps/calendar/ |
| Contacts | docs/design/contacts-rule0.md | shell/core/src/apps/contacts.rs, apps/contacts/ |
| Tasks | docs/design/tasks-rule0.md | shell/core/src/apps/tasks.rs, apps/tasks/ |
| Notes | docs/design/notes-rule0.md | shell/core/src/apps/notes.rs, apps/notes/ |
| Calculator | docs/design/calculator-rule0.md | shell/core/src/apps/calculator.rs, apps/calculator/ |
