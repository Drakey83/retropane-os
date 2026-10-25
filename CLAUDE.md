# Working on RetroPane OS (read at the start of every session)

0. **The owner's number one rule: iterate, don't imitate; be clever,
   modern and efficient.** Before designing or building anything (a
   driver, a filter, a test, a table of values), write down in the plan
   or the commit: what Dell, Windows Mobile, Linux or the textbook does
   (a finding, never the answer); the best way today's tools allow
   (computed offline, measured, fitted to this exact hardware); and why
   ours beats the original. A textbook default or a copied value is a
   first draft to beat, not a design. The owner should never have to ask
   for this again (2026-10-25: the scaler filter started as a 1980s cubic
   until he asked). **Enforced, not only printed:** `tools/remind/iterate_gate.py`
   (a PreToolUse hook in `.claude/settings.json`) refuses any commit holding
   code whose message lacks an `Iterate:` line (60+ characters: vendor or
   textbook way; ours; why it wins); docs-only commits may say
   `Iterate: n/a`. The printed reminders (`remind.py`) only show the rule;
   the gate is what makes skipping it impossible.
1. **Read [docs/ROADMAP.md](docs/ROADMAP.md) "Now: the gate" before
   choosing any work or answering a progress question.** The gate holds
   the architectural lines (`arch N` in the ledgers of
   [docs/research/2700g-full-potential.md](docs/research/2700g-full-potential.md)
   and [docs/research/cpu-full-potential.md](docs/research/cpu-full-potential.md))
   phase by phase: work waits only when its design depends on an open
   architectural line, and you name that line ("the gate" alone is not a
   reason). Optimizations are worked from
   [docs/research/backlog.md](docs/research/backlog.md), top first. Report
   the count from `cargo xtask gate` (never from memory) in every summary,
   change a ledger row only with the run (or the owner's ruling) that shows
   it, and add no architectural line without the owner's yes.
2. Before answering "have we done X?", look in the docs that plan X (the
   roadmap, the two full-potential pages, `docs/quality-audit.md`, the
   ADRs) instead of answering from memory.
3. A plan written down is a plan to follow: when a session's work is
   decided, the roadmap's step list is where it goes, and where the next
   session picks it up.
4. **Nothing parked is lost:** anything set aside, whether the owner
   raised it or you found it, goes on [docs/reminders.md](docs/reminders.md)
   in the same reply, with a trigger (`device-run`, `image:rp-<name>`,
   `after:<what>`, `date:`, `session`). The hooks print the open ones at
   session start and before every device run or test-image build: fold the
   due ones into that run, or say why not. Every summary names the
   reminders that came due; a row closes only with its commit or run
   (`done <ref>`), or the owner's reason (`dropped <reason>`).
