# Reports: how they are written

The owner's lists end in a report here (`deep-dive-<date>-fix.md`) and a
decisions file (`decisions-<date>.md`). Three reports in a row went stale
on what GitHub held (2026-11-07, 11-09, 11-10), each found by the next
list. The rule below is what the next report-writer follows; it is
linked from CLAUDE.md so every session reads it.

## Push, merge and CI claims

1. **A push claim is written last**, after the push state is settled, and
   checked against GitHub just before the report is finished:
   `git ls-remote origin` for the branch and `main`, and
   `git merge-base --is-ancestor <commit> origin/main` for every commit
   the report cites.
2. **A report can't record a merge that hasn't happened.** When a push or
   merge lands after the report is written (the owner's "push to live",
   "Yes, push", "Yes, merge"), the same session adds a dated one-line
   **merge record** to that report in a follow-up commit: what moved
   (`main` from X to Y, or the branch pushed), on whose word, and how it
   was checked. If the report can't be committed then (a branch frozen
   for review), the record goes in that day's decisions file instead,
   under the box that asked for the push.
3. **A push claim is settled only when the merge is recorded.** Until
   then the report says what was true when written ("pull request 13
   open, main at 04d3d142"), in the past tense with its date, never
   "main not moved" as if it stays so.
4. **Every later report checks the earlier ones' headers.** A list's
   final report re-runs rule 1 over every report in this folder and fixes
   any header GitHub has overtaken, with a dated "since" line.
5. **A CI claim names which CI:** `cargo xtask ci` on this PC (Windows),
   or GitHub's workflow run (Linux), with the run's link and conclusion.
   A run still in progress is reported as in progress, with its link,
   never as passed.
