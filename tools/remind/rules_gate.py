"""The owner's hard rules, enforced before the tool call (CLAUDE.md rule 0).

    rules_gate.py bash           a PreToolUse hook for Bash: checks `git
                                 commit` and `git push` (stdin: the hook's JSON)
    rules_gate.py spawn          a PreToolUse hook for the tools that start
                                 another session or agent
    rules_gate.py commit-msg F   git's commit-msg hook (tools/githooks)
    rules_gate.py pre-push       git's pre-push hook (tools/githooks)
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
3. **ADR before code**: a commit touching a path in docs/adr-areas.md
   names at least one of its ADRs (`ADR-0034`).
4. **CI before push**: `git push` only when `cargo xtask ci` passed on
   exactly this HEAD with no tracked change (its stamp, target/ci-pass.txt).
   The emulator fork (`Drakey83/retropane-qemu`) has no `cargo xtask ci`:
   a push there is allowed only as a release (the owner, 2026-10-26: "I
   want you to do it"): a fast-forward of its release branch `retropane`,
   or a new `rp-v*` tag on a commit that branch already holds on GitHub;
   nothing forced, deleted or pushed to another branch (`fork_push_verdict`).
5. **One session**: starting another session or agent only when the
   owner's latest message asks for one.

The files a commit holds: those staged now, those named by a `git add` in
the same command, and with `-a` every tracked change; when they can't be
told, the strictest rule applies. A broken gate refuses, loudly.

Which repository: the one the command's git acts in, not the one this
script sits in. The PreToolUse hook always runs the main checkout's copy,
so a commit or push in a worktree (`git -C <dir>`, `cd <dir> && git ...`)
is judged by that worktree's HEAD, staged files and CI stamp
(`acting_dir`; until 2026-10-25 it read the main checkout's, and refused a
worktree push whose own CI had passed: reminder R39). Git's own hooks run
the worktree's copy and judge it already.
"""

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MIN_TEXT = 60
CODE = re.compile(r"\.(rs|py|c|h|s|S|ld|toml|sh|ps1|json)$")
COMMIT = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+(?:\"[^\"]*\"|'[^']*'|\S+))?\s+commit\b")
PUSH = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+(?:\"[^\"]*\"|'[^']*'|\S+))?\s+push\b")
ADD = re.compile(r"(?:^|[;&|(]\s*|\s)git(?:\s+-C\s+(?:\"[^\"]*\"|'[^']*'|\S+))?\s+(?:add|mv|rm)\s+([^;&|]*)")
LINE = re.compile(r"Iterate:\s*(.*)", re.S)
ATTRIBUTION = re.compile(r"co-authored-by|generated with", re.I)
ADR_CITED = re.compile(r"ADR-0*(\d{1,4})\b")
GIT_C = re.compile(r"(?:^|[;&|(]\s*|\s)git\s+-C\s+(\"[^\"]*\"|'[^']*'|\S+)")
CD = re.compile(r"(?:^|[;&|(]\s*)cd\s+(\"[^\"]*\"|'[^']*'|[^\s;&|]+)")
# The repository the command being judged acts in (set by `hook`).
REPO = ROOT
ASKS_FOR_SPAWN = re.compile(
    r"\b(sessions?|agents?|subagents?|workflows?|fan[- ]?out|in parallel|fork)\b", re.I
)


def native(path):
    """A Git Bash path (`/c/Users/...`) as Windows' Python opens it; others
    unchanged; quotes dropped."""
    path = path.strip("'\"")
    m = re.match(r"^/([a-zA-Z])(/|$)", path)
    return f"{m.group(1).upper()}:/{path[3:]}" if m else path


def acting_dir(command, cwd):
    """Where the command's `git commit` or `git push` runs: the folder of a
    `git -C <dir>` on it, else the `cd`s before it (each from the last),
    else the hook's working directory, else this script's repository."""
    here = native(cwd) if cwd else ROOT
    act = COMMIT.search(command) or PUSH.search(command)
    end = act.start() if act else len(command)
    seg = command[act.start():act.end()] if act else ""
    m = GIT_C.search(seg)
    for c in CD.finditer(command[:end]):
        here = os.path.join(here, native(c.group(1)))
    if m:
        here = os.path.join(here, native(m.group(1)))
    return os.path.normpath(here)


def repo_of(folder):
    """The top of the repository holding `folder`."""
    out = subprocess.run(
        ["git", "-C", folder, "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        timeout=10,
    )
    if out.returncode != 0:
        raise RuntimeError(f"{folder}: {out.stderr.strip()}")
    return out.stdout.strip()


def git(*args):
    out = subprocess.run(["git", *args], cwd=REPO, capture_output=True, text=True, timeout=10)
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
    """docs/adr-areas.md's rows: (path prefix, set of ADR numbers)."""
    rows = []
    path = os.path.join(REPO, "docs", "adr-areas.md")
    if not os.path.exists(path):  # a branch from before the table moved
        path = os.path.join(ROOT, "docs", "adr-areas.md")
    with open(path, encoding="utf-8") as fh:
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
                f"ADR before code: {p} is decided by {names} (docs/adr-areas.md); read its numbered "
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


FORK_URL = re.compile(r"github\.com[/:]Drakey83/retropane-qemu(\.git)?/?$", re.I)
RELEASE_BRANCH = "refs/heads/retropane"
RELEASE_TAG = re.compile(r"^(refs/tags/)?rp-v\d+\.\d+\.\d+$")


def push_words(command):
    """The words after the command's push (to its end or the next `;`,
    `&` or `|`)."""
    m = PUSH.search(command)
    rest = re.split(r"[;&|]", command[m.end():], maxsplit=1)[0]
    return [w.strip("'\"") for w in rest.split()]


def fork_push_verdict(words, remote_url, remote_head, remote_has_tag, is_ancestor, resolve):
    """A push to the emulator fork, judged as a release: None (not the
    fork) or "" to allow, else the reason to refuse. `words` follow the
    push; `remote_head` is the remote's release branch; `remote_has_tag`,
    `is_ancestor(a, b)` and `resolve(rev)` (a commit, or None) ask git."""
    if not FORK_URL.search(remote_url or ""):
        return None  # not the fork: the CI rule decides
    if any(w.startswith("-") for w in words):
        return "fork push: no options (force, delete, mirror, all, tags): name the remote and what to push"
    if len(words) < 2:
        return "fork push: name the remote and exactly what to push (the release branch or an rp-v* tag)"
    for spec in words[1:]:
        if spec.startswith("+"):
            return f"fork push: `{spec}` forces: only fast-forwards"
        src, _, dst = spec.partition(":")
        dst = dst or src
        if RELEASE_TAG.match(src) and RELEASE_TAG.match(dst) and src.split("/")[-1] == dst.split("/")[-1]:
            tag = dst.split("/")[-1]
            target = resolve(f"refs/tags/{tag}^{{}}")
            if target is None:
                return f"fork push: no local tag {tag}"
            if remote_has_tag(tag):
                return f"fork push: {tag} is already published (a release is never moved)"
            if remote_head is None or not is_ancestor(target, remote_head):
                return f"fork push: {tag} is on a commit the release branch doesn't hold yet: push the branch first"
            continue
        if dst in ("retropane", RELEASE_BRANCH):
            commit = resolve(src)
            if commit is None:
                return f"fork push: `{src}` names no commit"
            if remote_head is not None and not is_ancestor(remote_head, commit):
                return "fork push: not a fast-forward of the release branch"
            continue
        return f"fork push: `{spec}`: only the release branch `retropane` or an rp-v* tag"
    return ""


def read_stamp():
    try:
        with open(os.path.join(REPO, "target", "ci-pass.txt"), encoding="utf-8") as fh:
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


def remote_ref(remote, ref):
    """What `remote` has at `ref` on the server (GitHub), or None."""
    out = subprocess.run(
        ["git", "ls-remote", remote, ref], cwd=REPO, capture_output=True, text=True, timeout=60
    )
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip())
    line = out.stdout.split("\n")[0].split()
    return line[0] if line else None


def resolve(rev):
    out = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", rev], cwd=REPO, capture_output=True, text=True, timeout=10
    )
    return out.stdout.strip() or None


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
    if not (COMMIT.search(command) or PUSH.search(command)):
        return
    global REPO
    REPO = repo_of(acting_dir(command, data.get("cwd")))
    if COMMIT.search(command):
        try:
            paths = files_in(command)
        except Exception:
            paths = None
        reason = commit_verdict(command, paths, areas())
        if reason:
            deny(reason)
    if PUSH.search(command):
        words = push_words(command)
        remote = next((w for w in words if not w.startswith("-")), "origin")
        try:
            url = git("remote", "get-url", remote)[0]
        except RuntimeError:
            url = ""
        if FORK_URL.search(url):
            reason = fork_push_verdict(
                words,
                url,
                remote_ref(remote, RELEASE_BRANCH),
                lambda tag: remote_ref(remote, f"refs/tags/{tag}") is not None,
                lambda a, b: subprocess.run(
                    ["git", "merge-base", "--is-ancestor", a, b], cwd=REPO, timeout=10
                ).returncode
                == 0,
                resolve,
            )
            if reason:
                deny(reason)
            return
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
        ('git commit -m "d\n\nIterate: n/a"', {"docs/adr-areas.md"}, True),
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
    if (
        not COMMIT.search("cd x && git -C . commit -q -m y")
        or not PUSH.search("git push origin main")
        # A quoted folder with a space once let a push past the patterns.
        or not PUSH.search('git -C "C:/a b/wt" push origin x')
        or not COMMIT.search("git -C 'C:/a b' commit -m y")
    ):
        bad += 1
        print("WRONG: missed a commit or push")
    pushes = [
        (("a" * 40, True, ("a" * 40, "clean")), True),
        (("a" * 40, True, ("b" * 40, "clean")), False),
        (("a" * 40, False, ("a" * 40, "clean")), False),
        (("a" * 40, True, ("a" * 40, "dirty")), False),
        (("a" * 40, True, None), False),
    ]
    # Which repository: the git -C folder, the cds before the git command
    # (chained, relative to the last), the hook's folder, Git Bash paths.
    dirs = [
        ("git push origin x", "C:/r", "C:/r"),
        ("git -C /c/Users/a/wt push origin x", "C:/r", "C:/Users/a/wt"),
        ('git -C "C:/a b/wt" push', "C:/r", "C:/a b/wt"),
        ("cd /c/Users/a/wt && git push origin x", "C:/r", "C:/Users/a/wt"),
        ("cd a && cd b && git commit -m x", "C:/r", "C:/r/a/b"),
        ("git push origin x && cd /c/elsewhere", "C:/r", "C:/r"),
        ("cd /c/w && git -C ../v commit -m x", "C:/r", "C:/v"),
        ("git push", None, ROOT),
    ]
    for cmd, cwd, want in dirs:
        got = acting_dir(cmd, cwd)
        if os.path.normcase(got) != os.path.normcase(os.path.normpath(want)):
            bad += 1
            print("WRONG folder:", cmd, "->", got)
    for (head, clean, stamp), allow in pushes:
        if (push_verdict(head, clean, stamp) is None) != allow:
            bad += 1
            print("WRONG push:", head[:4], clean, stamp)
    # The emulator fork: releases only (a fast-forward of `retropane`, a new
    # rp-v* tag on a commit it holds), judged on a fake history c1 <- c2.
    fork = "https://github.com/Drakey83/retropane-qemu.git"
    commits_ = {"c1": "c1", "c2": "c2", "refs/tags/rp-v0.31.0^{}": "c2", "refs/tags/rp-v0.32.0^{}": "c3"}
    anc = lambda a, b: a == b or (a, b) == ("c1", "c2")
    published = lambda tag: tag == "rp-v0.30.0"
    forks = [
        (["fork", "c2:refs/heads/retropane"], "c1", True),
        (["fork", "c2:retropane"], "c1", True),
        (["fork", "c1:refs/heads/retropane"], "c2", False),  # backwards
        (["fork", "+c2:refs/heads/retropane"], "c1", False),
        (["-f", "fork", "c2:refs/heads/retropane"], "c1", False),
        (["fork", "--delete", "retropane"], "c1", False),
        (["fork", "c2:refs/heads/rp-next"], "c1", False),
        (["fork", "rp-v0.31.0"], "c2", True),
        (["fork", "rp-v0.31.0"], "c1", False),  # the branch not pushed yet
        (["fork", "rp-v0.32.0"], "c2", False),  # a tag off the branch
        (["fork", "rp-v0.30.0"], "c2", False),  # already published
        (["fork", "rp-v0.31.0:rp-v0.99.0"], "c2", False),
        (["fork"], "c2", False),
        (["--tags", "fork"], "c2", False),
    ]
    for words, head, allow in forks:
        got = fork_push_verdict(words, fork, head, published, anc, commits_.get) in (None, "")
        if got != allow:
            bad += 1
            print("WRONG fork push:", words, head, "allowed" if got else "refused")
    os_url = "https://github.com/Drakey83/retropane-os.git"
    if fork_push_verdict(["origin", "x"], os_url, None, published, anc, commits_.get) is not None:
        bad += 1
        print("WRONG: the fork's rule judged another repository")
    if push_words("cd /c/q && git " + "push fork rp-v0.31.0 && echo ok") != ["fork", "rp-v0.31.0"]:
        bad += 1
        print("WRONG: push words")
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


def git_hook(mode):
    """Git's own commit-msg and pre-push hooks (tools/githooks): the same
    rules at the moment git acts, with the files really staged (a file
    changed by the same command as the commit reaches git, not the
    PreToolUse check). Claude's commands only (`CLAUDECODE` set): the
    owner's own commits and pushes pass. Exit 1 refuses."""
    if not os.environ.get("CLAUDECODE"):
        return 0
    if mode == "commit-msg":
        with open(sys.argv[2], encoding="utf-8") as fh:
            message = fh.read()
        reason = commit_verdict(message, set(git("diff", "--cached", "--name-only")), areas())
    else:
        head = git("rev-parse", "HEAD")[0]
        clean = not git("status", "--porcelain", "--untracked-files=no")
        reason = push_verdict(head, clean, read_stamp())
    if reason:
        print("Refused by the owner's hard rules (tools/remind/rules_gate.py): " + reason, file=sys.stderr)
        return 1
    return 0


def main():
    if "--self-test" in sys.argv:
        return self_test()
    mode = sys.argv[1] if len(sys.argv) > 1 else "bash"
    if mode in ("commit-msg", "pre-push"):
        try:
            return git_hook(mode)
        except Exception as e:  # a broken gate refuses, loudly
            print(f"rules_gate.py failed ({e}): fix the gate, don't bypass it", file=sys.stderr)
            return 1
    try:
        sys.stdout.reconfigure(encoding="utf-8")
        hook(mode)
    except Exception as e:  # a broken gate refuses, loudly
        deny(f"rules_gate.py failed ({e}): fix the gate, don't bypass it")
    return 0


if __name__ == "__main__":
    sys.exit(main())
