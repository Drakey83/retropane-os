"""The owner's number one rule, enforced at commit time (CLAUDE.md rule 0).

    iterate_gate.py              a PreToolUse hook: reads the hook's JSON on
                                 stdin; refuses a Bash `git commit` whose
                                 message has no `Iterate:` line when the
                                 commit holds code
    iterate_gate.py --self-test  checks its own rules

The owner, 2026-10-25: "why do i have to keep repeating this, it should be
the number one rule." The standing rule was printed before every build and
run, yet a textbook filter was still built: a reminder at build time comes
after the design. So every commit that holds code must say, in its message:

    Iterate: <what Dell, Windows Mobile, Linux or the textbook does>;
    <the better way chosen>; <why it wins (measured, or how it will be)>

at least MIN_TEXT characters after `Iterate:`. A commit with no code (docs,
reminders, the ledgers) may say `Iterate: n/a` instead. The files are the
ones staged now, those named by a `git add` in the same command, and with
`-a` every tracked change; when they can't be told, the full line is
required. Only Claude's commands pass through this hook: the owner's own
commits are never checked.
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
ADD = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+\S+)?\s+add\s+([^;&|]*)")
LINE = re.compile(r"Iterate:\s*(.*)", re.S)


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
    if re.search(r"\scommit\b[^;&|]*\s-[a-zA-Z]*a", command):
        paths.update(git("diff", "--name-only"))
    return paths


def verdict(command, paths):
    """None to allow, or the reason to refuse."""
    if not COMMIT.search(command):
        return None
    m = LINE.search(command)
    text = m.group(1).strip().strip("\"'") if m else ""
    has_code = paths is None or any(CODE.search(p) for p in paths)
    if m and text.lower().startswith("n/a") and not has_code:
        return None
    if m and len(text) >= MIN_TEXT and not text.lower().startswith("n/a"):
        return None
    if not m:
        why = "the message has no `Iterate:` line"
    elif text.lower().startswith("n/a"):
        why = "`Iterate: n/a` is only for commits with no code, and this one holds code"
    else:
        why = f"the `Iterate:` line is too short ({len(text)} characters, at least {MIN_TEXT})"
    return (
        "Refused by the owner's number one rule (CLAUDE.md rule 0): " + why + ". "
        "Add to the commit message: `Iterate: <what Dell, WM, Linux or the textbook does>; "
        "<the better way chosen>; <why it wins>`. Design it that way first if it wasn't."
    )


def hook():
    data = json.load(sys.stdin)
    command = (data.get("tool_input") or {}).get("command") or ""
    if not COMMIT.search(command):
        return
    try:
        paths = files_in(command)
    except Exception:
        paths = None
    reason = verdict(command, paths)
    if reason:
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        }))


def self_test():
    long = "Linux ships one upscale table; ours is least-squares per scale, measured on the X51v"
    cases = [
        ("ls -la", {"a.rs"}, True),
        ("git status", None, True),
        ('git commit -m "fix"', {"drivers/x.rs"}, False),
        (f'git commit -m "fix\n\nIterate: {long}"', {"drivers/x.rs"}, True),
        ('git commit -m "x\n\nIterate: too short"', {"drivers/x.rs"}, False),
        ('git commit -m "docs\n\nIterate: n/a (docs only)"', {"docs/a.md"}, True),
        ('git commit -m "docs\n\nIterate: n/a (docs only)"', {"docs/a.md", "tools/t.py"}, False),
        ('git add docs && git commit -m "d\n\nIterate: n/a"', {"docs/a.md"}, True),
        ('cd x && git -C . commit -q -m "no line"', {"a.md"}, False),
        ('git commit -m "Iterate: n/a"', None, False),
        ('grep -n "git commit" notes.txt', None, True),
    ]
    bad = 0
    for cmd, paths, allow in cases:
        got = verdict(cmd, paths) is None
        if got != allow:
            bad += 1
            print("WRONG:", repr(cmd), paths, "allowed" if got else "refused")
    print("iterate_gate self-test:", "ok" if not bad else f"{bad} wrong")
    return 1 if bad else 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        hook()
    except Exception as e:  # a broken gate refuses commits, loudly
        print(json.dumps({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": f"iterate_gate.py failed ({e}): fix the gate, don't bypass it",
            }
        }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
