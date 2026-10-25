"""The owner's hard rules, enforced before the tool call (CLAUDE.md rule 0).

    rules_gate.py bash           a PreToolUse hook for Bash: checks `git
                                 commit` and `git push` (stdin: the hook's JSON)
    rules_gate.py spawn          a PreToolUse hook for the tools that start
                                 another session or agent
    rules_gate.py --self-test    checks its own rules

The owner, 2026-10-25, after a printed reminder was skipped: "why do i have
to keep repeating this, it should be the number one rule" and "HARD GATE
WAS PROMISED TO ME". A reminder prints and can be skimmed; this refuses.
Only Claude's tool calls pass through it: the owner's own commands are
never checked.

1. **Iterate, don't imitate** (rule 0): a commit holding code needs an
   `Iterate:` line in its message, at least MIN_TEXT characters: what Dell,
   Windows Mobile, Linux or the textbook does; the better way chosen; why
   it wins. A commit with no code may say `Iterate: n/a`.
2. **No attribution**: a commit message naming a co-author or a generator
   (`Co-Authored-By`, `Generated with`) is refused (the owner's rule).
3. **ADR before code**: a commit touching a path in docs/adr/areas.md
   names at least one of its ADRs (`ADR-0034`).
4. **CI before push**: `git push` only when `cargo xtask ci` passed on
   exactly this HEAD with no tracked change (its stamp, target/ci-pass.txt).
5. **One session**: starting another session or agent only when the
   owner's latest message asks for one.

The files a commit holds: those staged now, those named by a `git add` in
the same command, and with `-a` every tracked change; when they can't be
told, the strictest rule applies. A broken gate refuses, loudly.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MIN_TEXT = 60
CODE = re.compile(r"\.(rs|py|c|h|s|S|ld|toml|sh|ps1|json)$")
COMMIT = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+\S+)?\s+commit\b")
PUSH = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+\S+)?\s+push\b")
ADD = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+\S+)?\s+(?:add|mv|rm)\s+([^;&|]*)")
LINE = re.compile(r"Iterate:\s*(.*)", re.S)
ATTRIBUTION = re.compile(r"co-authored-by|generated with", re.I)
ADR_CITED = re.compile(r"ADR-0*(\d{1,4})\b")
STAMP = os.path.join(ROOT, "target", "ci-pass.txt")
ASKS_FOR_SPAWN = re.compile(
    r"\b(sessions?|agents?|subagents?|workflows?|fan[- ]?out|in parallel|fork)\b", re.I
)


def git(*args):
    out = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, timeout=10)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip())
    return [l for l in out.stdout.splitlines() if l.strip()]


def files_in(command):
    """The paths this commit will hold, or None when they can't be told."""
    paths = set(git("diff", "--cached", "--name-only"))
    for m in ADD.finditer(command):
        args = [a.strip("'\"") for a in m.group(1).split() if not a.startswith("-")]
        if not args:
            continue
        paths.update(git("ls-files", "--modified", "--others", "--exclude-standard", "--", *args))
        paths.update(git("diff", "--name-only", "--", *args))
        paths.update(git("diff", "--cached", "--name-only", "--", *args))
    if re.search(r"\scommit\b[^;&|]*\s-[a-zA-Z]*a", command):
        paths.update(git("diff", "--name-only"))
    return paths


def areas():
    """docs/adr/areas.md's rows: (path prefix, set of ADR numbers)."""
    rows = []
    with open(os.path.join(ROOT, "docs", "adr", "areas.md"), encoding="utf-8") as fh:
        for line in fh:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if len(cells) == 2 and "/" in cells[0] and re.fullmatch(r"[\d, ]+", cells[1]):
                rows.append((cells[0], {int(n) for n in cells[1].split(",") if n.strip()}))
    return rows


def adrs_needed(paths, rows):
    """For each touched path in an ADR-decided area, its ADR set."""
    need = {}
    for p in paths:
        best = max((r for r in rows if p.startswith(r[0])), key=lambda r: len(r[0]), default=None)
        if best:
            need[p] = best[1]
    return need


def commit_verdict(command, paths, rows):
    """None to allow, or the reason to refuse."""
    m = LINE.search(command)
    text = m.group(1).strip().strip("\"'") if m else ""
    has_code = paths is None or any(CODE.search(p) for p in paths)
    if ATTRIBUTION.search(command):
        return "the message names a co-author or a generator (the owner's rule: commit messages carry only the change)"
    if not m:
        return (
            "rule 0: the message has no `Iterate:` line. Add `Iterate: <what Dell, WM, Linux or the "
            "textbook does>; <the better way chosen>; <why it wins>` (design it that way first if it wasn't)"
        )
    if text.lower().startswith("n/a"):
        if has_code:
            return "rule 0: `Iterate: n/a` is only for commits with no code, and this one holds code"
    elif len(text) < MIN_TEXT:
        return f"rule 0: the `Iterate:` line is too short ({len(text)} characters, at least {MIN_TEXT})"
    if paths is not None:
        cited = {int(n) for n in ADR_CITED.findall(command)}
        missing = {p: s for p, s in adrs_needed(paths, rows).items() if not s & cited}
        if missing:
            p, s = sorted(missing.items())[0]
            names = ", ".join(f"ADR-{n:04}" for n in sorted(s))
            return (
                f"ADR before code: {p} is decided by {names} (docs/adr/areas.md); read its numbered "
                f"rules, check the change against them, and name the ADR in the message"
            )
    return None


def push_verdict(head, clean, stamp):
    if stamp is None:
        return "CI before push: `cargo xtask ci` has not passed here (no target/ci-pass.txt)"
    s_head, s_state = stamp
    if s_head != head:
        return f"CI before push: CI last passed on {s_head[:8]}, HEAD is {head[:8]}: run `cargo xtask ci` again"
    if s_state != "clean" or not clean:
        return "CI before push: CI passed with tracked changes uncommitted, or there are some now: commit, then run `cargo xtask ci`"
    return None


def read_stamp():
    try:
        with open(STAMP, encoding="utf-8") as fh:
            parts = fh.read().split()
        return (parts[0], parts[1]) if len(parts) >= 2 else None
    except OSError:
        return None


def last_owner_message(transcript):
    """The owner's latest typed message in the session's transcript."""
    last = None
    with open(transcript, encoding="utf-8") as fh:
        for line in fh:
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if o.get("type") != "user" or o.get("isMeta"):
                continue
            c = (o.get("message") or {}).get("content")
            texts = [c] if isinstance(c, str) else [
                x.get("text", "") for x in (c or []) if isinstance(x, dict) and x.get("type") == "text"
            ]
            texts = [t for t in texts if t.strip() and not t.lstrip().startswith("<")]
            if texts:
                last = "\n".join(texts)
    return last


def spawn_verdict(message):
    if message is None:
        return "one session: the owner's latest message couldn't be read"
    if ASKS_FOR_SPAWN.search(message):
        return None
    return (
        "one session (the owner's hard rule, credits): another session or agent only when his "
        "latest message asks for one; do the work here"
    )


def deny(reason):
    print(json.dumps({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "Refused by the owner's hard rules (tools/remind/rules_gate.py): " + reason,
        }
    }))


def hook(mode):
    data = json.load(sys.stdin)
    if mode == "spawn":
        try:
            msg = last_owner_message(data.get("transcript_path") or "")
        except OSError:
            msg = None
        reason = spawn_verdict(msg)
        if reason:
            deny(reason)
        return
    command = (data.get("tool_input") or {}).get("command") or ""
    if COMMIT.search(command):
        try:
            paths = files_in(command)
        except Exception:
            paths = None
        reason = commit_verdict(command, paths, areas())
        if reason:
            deny(reason)
    if PUSH.search(command):
        head = git("rev-parse", "HEAD")[0]
        clean = not git("status", "--porcelain", "--untracked-files=no")
        reason = push_verdict(head, clean, read_stamp())
        if reason:
            deny(reason)


def self_test():
    long = "Linux ships one upscale table; ours is least-squares per scale, measured on the X51v"
    rows = [("drivers/mbx-display/", {22, 34}), ("drivers/mved/", {25})]
    commits = [
        ('git commit -m "fix"', {"drivers/x.rs"}, False),
        (f'git commit -m "fix\n\nIterate: {long}"', {"drivers/x.rs"}, True),
        ('git commit -m "x\n\nIterate: too short"', {"drivers/x.rs"}, False),
        ('git commit -m "docs\n\nIterate: n/a (docs only)"', {"docs/a.md"}, True),
        ('git commit -m "docs\n\nIterate: n/a"', {"docs/a.md", "tools/t.py"}, False),
        ('git commit -m "Iterate: n/a"', None, False),
        (f'git commit -m "x\n\nIterate: {long}\n\nCo-Authored-By: someone"', {"a.rs"}, False),
        (f'git commit -m "x\n\nIterate: {long}\n\nGenerated with a tool"', {"a.rs"}, False),
        # ADR before code: an area's file needs one of its ADRs named.
        (f'git commit -m "x\n\nIterate: {long}"', {"drivers/mbx-display/src/a.rs"}, False),
        (f'git commit -m "x (ADR-0034)\n\nIterate: {long}"', {"drivers/mbx-display/src/a.rs"}, True),
        (f'git commit -m "x (ADR-0025)\n\nIterate: {long}"', {"drivers/mbx-display/src/a.rs"}, False),
        (f'git commit -m "x ADR-0034\n\nIterate: {long}"', {"drivers/mbx-display/a.rs", "drivers/mved/b.rs"}, False),
        (f'git commit -m "x ADR-0034 ADR-0025\n\nIterate: {long}"', {"drivers/mbx-display/a.rs", "drivers/mved/b.rs"}, True),
        ('git commit -m "d\n\nIterate: n/a"', {"docs/adr/areas.md"}, True),
    ]
    bad = 0
    for cmd, paths, allow in commits:
        got = commit_verdict(cmd, paths, rows) is None
        if got != allow:
            bad += 1
            print("WRONG commit:", repr(cmd[:60]), paths, "allowed" if got else "refused")
    for cmd in ['grep -n "git commit" notes.txt', "ls", "git status"]:
        if COMMIT.search(cmd) or PUSH.search(cmd):
            bad += 1
            print("WRONG: matched", cmd)
    if not COMMIT.search("cd x && git -C . commit -q -m y") or not PUSH.search("git push origin main"):
        bad += 1
        print("WRONG: missed a commit or push")
    pushes = [
        (("a" * 40, True, ("a" * 40, "clean")), True),
        (("a" * 40, True, ("b" * 40, "clean")), False),
        (("a" * 40, False, ("a" * 40, "clean")), False),
        (("a" * 40, True, ("a" * 40, "dirty")), False),
        (("a" * 40, True, None), False),
    ]
    for (head, clean, stamp), allow in pushes:
        if (push_verdict(head, clean, stamp) is None) != allow:
            bad += 1
            print("WRONG push:", head[:4], clean, stamp)
    spawns = [
        ("Yes, build hard-rule checks 1-4 now.", False),
        ("start a side session for the CF card work", True),
        ("use a workflow to fan out the review", True),
        ("run it with subagents", True),
        ("go soak", False),
        (None, False),
    ]
    for msg, allow in spawns:
        if (spawn_verdict(msg) is None) != allow:
            bad += 1
            print("WRONG spawn:", msg)
    # The real table: every ADR it names exists, every prefix is a path.
    adr_dir = os.path.join(ROOT, "docs", "adr")
    have = {int(f[:4]) for f in os.listdir(adr_dir) if f[:4].isdigit()}
    for prefix, nums in areas():
        if not nums <= have or not os.path.exists(os.path.join(ROOT, prefix)):
            bad += 1
            print("WRONG areas.md row:", prefix, sorted(nums - have))
    print("rules_gate self-test:", "ok" if not bad else f"{bad} wrong")
    return 1 if bad else 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    mode = sys.argv[1] if len(sys.argv) > 1 else "bash"
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        hook(mode)
    except Exception as e:  # a broken gate refuses, loudly
        deny(f"rules_gate.py failed ({e}): fix the gate, don't bypass it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
