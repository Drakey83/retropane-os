"""Reminders from docs/reminders.md, printed for Claude Code's hooks.

    remind.py session    the open reminders, at a session's start
    remind.py pretool    reads a PreToolUse hook's JSON on stdin; when the
                         Bash command runs or builds a device image (`cargo
                         xtask load`, `codegen-variant`), the open reminders
                         whose trigger is `device-run` or names that image

Never blocks a tool call and never fails one: any error prints nothing and
exits 0. Standard library only; the table format is docs/reminders.md's,
checked in CI by `cargo xtask reminders`.
"""

import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LIST = os.path.join(ROOT, "docs", "reminders.md")


def rows():
    """The table's rows: (id, what, raised, triggers, status)."""
    out = []
    with open(LIST, encoding="utf-8") as f:
        for line in f:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if len(cells) == 5 and re.fullmatch(r"R\d+", cells[0]):
                triggers = [t.strip() for t in cells[3].split(";") if t.strip()]
                out.append((cells[0], cells[1], cells[2], triggers, cells[4]))
    return out


def is_open(row):
    return row[4] == "open"


def line(row):
    return f"- {row[0]} [{'; '.join(row[3])}] {row[1]}"


def standing():
    """The list's "Standing rules" bullets (the owner's rules that never
    close), joined into one line each."""
    rules, inside, current = [], False, None
    with open(LIST, encoding="utf-8") as f:
        for raw in f:
            text = raw.rstrip("\n")
            if text.startswith("## "):
                inside = text.strip() == "## Standing rules"
                continue
            if not inside:
                continue
            if text.startswith("- "):
                current = text[2:].strip()
                rules.append(current)
            elif text.strip() and rules and current is not None and text.startswith("  "):
                rules[-1] += " " + text.strip()
            elif not text.strip():
                current = None
    return rules


def standing_text():
    rules = standing()
    if not rules:
        return ""
    return "STANDING RULES (the owner's most important; docs/reminders.md):\n" + "\n".join(
        f"- {r}" for r in rules
    )


def session():
    rules = standing_text()
    if rules:
        print(rules)
    open_rows = [r for r in rows() if is_open(r)]
    if not open_rows:
        return
    print(
        f"Open reminders ({len(open_rows)}, docs/reminders.md): bring each up "
        "when its trigger comes; fold device-run ones into the next device run; "
        "anything newly parked goes on the list in the same reply."
    )
    for r in open_rows:
        print(line(r))


# A command that runs or builds a device image, and the image it names.
DEVICE = re.compile(r"xtask\s+(load|codegen-variant)\b")
IMAGE = re.compile(r"\b(rp-[a-z0-9][a-z0-9-]*)")


def pretool():
    data = json.load(sys.stdin)
    command = (data.get("tool_input") or {}).get("command") or ""
    if not DEVICE.search(command):
        return
    images = set(IMAGE.findall(command))
    due = [
        r
        for r in rows()
        if is_open(r)
        and any(t == "device-run" or t[len("image:"):] in images for t in r[3] if t == "device-run" or t.startswith("image:"))
    ]
    parts = []
    rules = standing_text()
    if rules:
        parts.append(
            rules
            + "\nFor this run: does it test our own better way, not only Dell's? Is any limit it assumes proven?"
        )
    if due:
        parts.append(
            "Reminders due for this device run or image build (docs/reminders.md): "
            "fold any that fit into this run, or say why not.\n" + "\n".join(line(r) for r in due)
        )
    if not parts:
        return
    text = "\n\n".join(parts)
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": text}}))


def main():
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        mode = sys.argv[1] if len(sys.argv) > 1 else "session"
        if mode == "session":
            session()
        elif mode == "pretool":
            pretool()
    except Exception:  # never block or fail a tool call
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
