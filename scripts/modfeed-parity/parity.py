#!/usr/bin/env python3
"""Runs two flockbook-modfeed binaries (the Swift one and the Rust one) on the same fixtures and compares them.

    scripts/modfeed-parity/parity.py --a <swift binary> --b <rust binary> [--report out.json]

Each binary gets the same, freshly made work folder: copies of the synthetic log fixtures, a `cache.json` that points
each fake terminal at its log (so no process scan runs), and a fake herdr-core (`fake_hc.py`). The same list of
commands runs against each (feed, older rows, status, card, approve, answer, send, errors). Their stdout is compared
line by line after dropping `end.ms`, and the keys and text each one sent to the fake herdr-core are compared too.
Some steps also have a fixed expected result and keys (`EXPECT`), checked on each side alone, and both sides must
make the same steps. Exit 0 when everything matches and every expectation holds, 1 otherwise. Never touches a real herdr-core; nothing here reads real logs.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FIXTURES = os.path.join(REPO, "Tests", "HerdrKitTests", "Fixtures")

# terminal address → (agent argv, fixture log). Logs come from Tests/HerdrKitTests/Fixtures (synthetic), plus
# `turn.jsonl` written below.
LOGS = {
    "t1": (["claude"], "session.jsonl"), "t2": (["claude"], "question.jsonl"), "t3": (["claude"], "queued.jsonl"),
    "t4": (["claude"], "injected.jsonl"), "t5": (["claude"], "pasted.jsonl"), "t6": (["codex"], "codex.jsonl"),
    "t7": (["codex"], "question-codex.jsonl"), "t8": (["pi"], "pi.jsonl"), "t9": (["grok"], "grok.jsonl"),
    "t10": (["agy"], "agy.jsonl"), "t11": (["agy"], "agy-order.jsonl"), "t12": (["agy"], "agy-injected.jsonl"),
    "t13": (["cursor-agent"], "cursor.jsonl"), "t20": (["claude"], "turn.jsonl"),
}


def ts(s):
    return "2026-11-03T01:%02d:%02d.000Z" % (40 + s // 60, s % 60)


def turn_log():
    """A Claude turn ending with a Bash call that has no result yet (the approval card's log call)."""
    def user(i, t, text):
        return {"type": "user", "uuid": i, "timestamp": ts(t), "message": {"role": "user", "content": text}}
    def say(i, t, text):
        return {"type": "assistant", "uuid": i, "timestamp": ts(t),
                "message": {"role": "assistant", "model": "claude-test-1", "content": [{"type": "text", "text": text}]}}
    def tool(i, t, call, cmd):
        return {"type": "assistant", "uuid": i, "timestamp": ts(t), "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": call, "name": "Bash", "input": {"command": cmd, "description": "Run it"}}]}}
    def result(i, t, call, out):
        return {"type": "user", "uuid": i, "timestamp": ts(t), "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": call, "content": out}]}}
    lines = [user("u1", 0, "Fix the login bug ✨ — 日本語も"), tool("a1", 5, "c1", "ls -la"), result("r1", 8, "c1", "ok\nmore"),
             say("a2", 20, "Looking at `auth.rs` now.\n\n```rust\nfn main() {}\n```"), user("u2", 1000, "Then remove the build"),
             tool("a3", 1003, "toolu_B", "rm -rf build")]
    return "\n".join(json.dumps(l, ensure_ascii=False) for l in lines) + "\n"


CLAUDE_BOX = ("✻ Working… 3s\n────────────────────\n Bash command\n Run it\n╌╌╌╌╌╌╌╌╌╌\n rm -rf build\n╌╌╌╌╌╌╌╌╌╌\n"
              " Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel")
QUESTION = ("❯ Ask me.\n────────────────────\n ☐ Color\nPick a color?\n❯ 1. Red (Recommended)\n     warm\n  2. Blue\n"
            "     cool\n  3. Type something.\n────────────────────\n  4. Chat about this\n"
            "Enter to select · ↑/↓ to navigate · Esc to cancel")


def grok_box(command, focus=2):
    rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed",
            "No, reject (type to add feedback)", "Never allow: echo"]
    l = ["  ❯ Write a file.", "", "  ◆ Working… 3.0s", "", "  ┃", "  ┃  ›  Run a command", "  ┃  │ " + command,
         "  ┃  ← → narrow scope", "  ┃"]
    l += ["  ┃  %d (%s) %s" % (i + 1, "●" if i + 1 == focus else "○", r) for i, r in enumerate(rows)]
    l += ["  ┃", "", "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback"]
    return "\n".join(l)


AGY_BOX = ("  > run the tests\n────────────────────────────\n\nRequesting permission for:\n  npm test\nRun this command?\n"
           "> 1. Yes, run command\n  2. Yes, and always allow\n  3. No, cancel\n\n↑/↓ Navigate · Enter Select")


def picker(questions):
    """Claude Code's picker in the fake herdr-core (fake_hc.py `new_picker`): (header, text, options, multi)."""
    qs = [{"header": h, "text": t, "options": [list(o) for o in opts], "multi": m} for h, t, opts, m in questions]
    return {"qs": qs, "at": 0, "focus": 0, "review": False, "done": False, "checks": [[] for _ in qs], "single": {}}


TWO_QUESTIONS = [("Color", "Pick a color?", [("Red", "warm"), ("Blue", "cool")], False),
                 ("Tops", "Which toppings?", [("Cheese", ""), ("Olive", ""), ("Basil", "")], True)]


def terminal(addr, argv, pid, status=None, kind=None):
    t = {"address": addr, "labels": {}, "argv": argv, "cwd": "/tmp/proj", "state": "live", "created_ms": 1,
         "title": "", "pid": pid, "parent": None, "foreground": None, "status": None}
    if status:
        t["status"] = {"state": status, **({"kind": kind} if kind else {})}
    return t


class Run:
    """One binary against a fresh work folder and fake herdr-core."""

    def __init__(self, binary, work):
        self.binary, self.work = binary, work
        if os.path.exists(work):
            shutil.rmtree(work)
        os.makedirs(os.path.join(work, "logs"))
        os.makedirs(os.path.join(work, "home"))
        self.state = os.path.join(work, "hc-state.json")
        self.calls = os.path.join(work, "calls.jsonl")
        self.sock = os.path.join(work, "hc.sock")
        self.cache = os.path.join(work, "cache", "cache.json")
        terms, entries = [], {}
        for i, (addr, (argv, name)) in enumerate(sorted(LOGS.items())):
            dst = os.path.join(work, "logs", name)
            if name == "turn.jsonl":
                open(dst, "w").write(turn_log())
            else:
                shutil.copy(os.path.join(FIXTURES, name), dst)
            pid = 5000 + i
            terms.append(terminal(addr, argv, pid, "idle"))
            entries[addr] = {"kind": {"cursor-agent": "cursor"}.get(argv[0], argv[0]), "cwd": "/tmp/proj", "pids": [pid],
                             "path": dst, "checked": time.time() + 3600}
        terms.append(terminal("t30", ["zsh"], 6000))
        # Claude Code's settings name Flockbook's status line wrapper; two sessions have a saved line (ANSI colours,
        # a full context, cost) in its folder.
        status_dir = os.path.join(work, "status line")
        os.makedirs(os.path.join(work, "home", ".claude"))
        json.dump({"statusLine": {"type": "command", "command": "'/opt/fb/flockbook-statusline' --dir '%s'" % status_dir}},
                  open(os.path.join(work, "home", ".claude", "settings.json"), "w"))
        os.makedirs(status_dir)
        records = {
            "session": ({"session_id": "session", "model": {"display_name": "Opus 9"}, "effort": {"level": "high"},
                         "context_window": {"used_percentage": 84}, "cost": {"total_cost_usd": 0.4237}},
                        "\x1b[32m~/proj\x1b[0m \x1b[2mmain\x1b[0m · own part"),
            "turn": ({"session_id": "turn", "model": {"display_name": "Sonnet 9", "id": "claude-sonnet-9"},
                      "context_window": {"used_percentage": 12}}, ""),
        }
        for sid, (inp, out) in records.items():
            json.dump(inp, open(os.path.join(status_dir, sid + ".json"), "w"))
            open(os.path.join(status_dir, sid + ".out"), "w").write(out)
        self.terms = terms
        self.write_state(terms, {})
        os.makedirs(os.path.dirname(self.cache))
        json.dump({"entries": entries, "feeds": {}}, open(self.cache, "w"))
        open(self.calls, "w").close()
        self.hc = subprocess.Popen([sys.executable, "-I", os.path.join(HERE, "fake_hc.py"), self.sock, self.state, self.calls],
                                   stdout=subprocess.PIPE, text=True)
        assert self.hc.stdout.readline().strip() == "ready"

    def write_state(self, terms, screens, after_keys=None, pickers=None, delay_keys=None):
        json.dump({"terminals": terms, "screens": screens, "after_keys": after_keys or {}, "pickers": pickers or {},
                   "delay_keys": delay_keys or {}}, open(self.state, "w"))

    def set_status(self, addr, status, kind):
        terms = [dict(t) for t in self.terms]
        for t in terms:
            if t["address"] == addr:
                t["status"] = {"state": status, "kind": kind}
        return terms

    def sent(self):
        out = []
        for l in open(self.calls):
            c = json.loads(l)
            if c["method"] in ("terminal.keys", "terminal.send", "terminal.put"):
                out.append(c)
        return out

    def cmd(self, *args):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": os.path.join(self.work, "home"),
               "HC_SOCKET": self.sock, "TZ": "UTC", "LANG": "en_US.UTF-8",
               # Swift's NSHomeDirectory() is the account's home, not $HOME: name Claude Code's folder for both.
               "CLAUDE_CONFIG_DIR": os.path.join(self.work, "home", ".claude")}
        p = subprocess.run([self.binary, *args], env=env, capture_output=True, timeout=60)
        lines = []
        for raw in p.stdout.decode("utf-8", "replace").splitlines():
            try:
                o = json.loads(raw)
            except ValueError:
                lines.append({"_unparsed": raw})
                continue
            if o.get("type") == "end":
                o.pop("ms", None)
            lines.append(o)
        return {"exit": p.returncode, "lines": lines, "raw": p.stdout.decode("utf-8", "replace")}

    def close(self):
        self.hc.kill()
        self.hc.wait()


def find(out, typ):
    return next((l for l in out["lines"] if l.get("type") == typ), {})


def scenario(r):
    """The commands, in order. Yields (name, output, keys and text sent by this command)."""
    def step(name, *args):
        before = len(r.sent())
        out = r.cmd(*args)
        return name, out, r.sent()[before:]

    C = ["--cache", r.cache]
    yield step("version", "version")
    yield step("usage: no command", "nope")
    yield step("usage: no terminal", "feed")
    yield step("no-terminal", "card", "--terminal", "t99", *C)
    yield step("not-agent", "card", "--terminal", "t30", *C)
    for addr, (_, name) in sorted(LOGS.items(), key=lambda kv: int(kv[0][1:])):
        first = step("feed %s (%s)" % (addr, name), "feed", "--terminal", addr, *C)
        yield first
        cursor = find(first[1], "log").get("cursor")
        if cursor:
            yield step("feed %s again (cursor)" % addr, "feed", "--terminal", addr, "--cursor", cursor, *C)
        log = find(first[1], "log")
        if log.get("path"):
            yield step("feed %s --before" % addr, "feed", "--terminal", addr, "--before", str(log["size"]), "--path", log["path"], *C)
        yield step("status %s" % addr, "status", "--terminal", addr, *C)

    # Approval, Claude: the log's pending call is the box on screen.
    r.write_state(r.set_status("t20", "blocked", "permission"), {"t20": CLAUDE_BOX})
    card = step("card t20 (approval, log call = box)", "card", "--terminal", "t20", *C)
    yield card
    req = find(card[1], "card").get("request", "")
    yield step("approve t20 wrong request", "approve", "--terminal", "t20", "--request", "screen-other", "--allow", *C)
    yield step("approve t20 deny", "approve", "--terminal", "t20", "--request", req, "--deny", *C)
    yield step("approve t20 again (answered)", "approve", "--terminal", "t20", "--request", req, "--allow", *C)
    yield step("card t20 (answered, box still drawn)", "card", "--terminal", "t20", *C)
    # Approval, Claude, no log call for it (another command on screen).
    r.write_state(r.set_status("t1", "blocked", "permission"), {"t1": CLAUDE_BOX.replace("rm -rf build", "ls")})
    card = step("card t1 (approval from the screen)", "card", "--terminal", "t1", *C)
    yield card
    yield step("approve t1 allow", "approve", "--terminal", "t1", "--request", find(card[1], "card").get("request", ""), "--allow", *C)
    # Grok.
    r.write_state(r.set_status("t9", "blocked", "permission"), {"t9": grok_box("echo two")}, {"t9": "  ❯ "})
    card = step("card t9 (grok approval)", "card", "--terminal", "t9", *C)
    yield card
    yield step("approve t9 allow (Enter on Yes, proceed)", "approve", "--terminal", "t9", "--request",
               find(card[1], "card").get("request", ""), "--allow", *C)
    # agy.
    r.write_state(r.set_status("t10", "blocked", "permission"), {"t10": AGY_BOX})
    card = step("card t10 (agy approval)", "card", "--terminal", "t10", *C)
    yield card
    yield step("approve t10 deny", "approve", "--terminal", "t10", "--request", find(card[1], "card").get("request", ""), "--deny", *C)
    # A Claude question picker on screen.
    r.write_state(r.set_status("t5", "blocked", "question"), {"t5": QUESTION}, {"t5": "⏺ User answered: Blue\n> "})
    card = step("card t5 (question picker)", "card", "--terminal", "t5", *C)
    yield card
    q = find(card[1], "card").get("request", "")
    yield step("answer t5 wrong request", "answer", "--terminal", "t5", "--request", "screen-nope", "--answers", '[{"picks":[1]}]', *C)
    yield step("answer t5 bad json", "answer", "--terminal", "t5", "--request", q, "--answers", "{}", *C)
    yield step("answer t5 picks [1]", "answer", "--terminal", "t5", "--request", q, "--answers", '[{"picks":[1]}]', *C)
    # No prompt: no card.
    r.write_state(r.terms, {"t6": "> ready"})
    yield step("card t6 (none)", "card", "--terminal", "t6", *C)
    # The composer.
    t1_log = os.path.join(r.work, "logs", "session.jsonl")
    yield step("send t1 another kind", "send", "--terminal", "t1", "--text", "hi", "--expect-kind", "codex", *C)
    yield step("send t1 another log", "send", "--terminal", "t1", "--text", "hi", "--expect-path", "/elsewhere.jsonl", *C)
    yield step("send t1 empty text", "send", "--terminal", "t1", "--text", "", *C)
    yield step("send t1", "send", "--terminal", "t1", "--text", "hello — ✓", "--expect-kind", "claude", "--expect-path", t1_log, *C)
    # A Claude picker with two questions (read from the screen): the card shows the first, one unread; --all walks the
    # picker with left/right (navigation only) and comes back; the answer picks Blue, then toggles Cheese and Basil.
    r.write_state(r.set_status("t3", "blocked", "question"), {}, pickers={"t3": picker(TWO_QUESTIONS)})
    card = step("card t3 (two questions)", "card", "--terminal", "t3", *C)
    yield card
    yield step("card t3 --all (walks the picker)", "card", "--terminal", "t3", "--all", *C)
    yield step("answer t3 two questions", "answer", "--terminal", "t3", "--request", find(card[1], "card").get("request", ""),
               "--answers", '[{"picks":[1]},{"picks":[0,2]}]', *C)
    # A key that hc holds (no reply within the sidecar's 8 s): unconfirmed, and the claim holds while still blocked.
    box = CLAUDE_BOX.replace("rm -rf build", "make clean")
    r.write_state(r.set_status("t20", "blocked", "permission"), {"t20": box}, delay_keys={"t20": 10})
    card = step("card t20 (make clean)", "card", "--terminal", "t20", *C)
    yield card
    req = find(card[1], "card").get("request", "")
    yield step("approve t20 allow, key held", "approve", "--terminal", "t20", "--request", req, "--allow", *C)
    r.write_state(r.set_status("t20", "blocked", "permission"), {"t20": box})
    yield step("approve t20 again while held", "approve", "--terminal", "t20", "--request", req, "--allow", *C)
    # Files in the composer. Local: the text goes as typed. On another machine: the file is put there first and the
    # text names that copy, quoted.
    pic = os.path.join(r.work, "shot one.png")
    open(pic, "wb").write(b"\x89PNG fake")
    r.write_state(r.terms, {})
    yield step("send t1 --file (local)", "send", "--terminal", "t1", "--text", "look at '%s'" % pic, "--file", pic, *C)
    remote = terminal("box/t40", ["claude"], 7000, "idle")
    r.write_state(r.terms + [remote], {})
    yield step("send box/t40 --file (remote)", "send", "--terminal", "box/t40", "--text", "look at '%s'" % pic, "--file", pic, *C)
    # An agent with no identity (no pid): nothing is sent.
    terms = [dict(t) for t in r.terms]
    for t in terms:
        if t["address"] == "t4":
            t["pid"], t["status"] = None, {"state": "blocked", "kind": "permission"}
    r.write_state(terms, {"t4": CLAUDE_BOX})
    card = step("card t4 (no identity)", "card", "--terminal", "t4", *C)
    yield card
    yield step("approve t4 (no identity)", "approve", "--terminal", "t4", "--request", find(card[1], "card").get("request", ""), "--allow", *C)
    yield step("send t4 (no identity)", "send", "--terminal", "t4", "--text", "hi", *C)


def brief(sent):
    """What a step sent: (method, keys, text or file name) per call."""
    return [(c["method"], c["params"].get("keys") or c["params"].get("text") or c["params"].get("name")) for c in sent]


WALK = [("terminal.keys", ["left"]), ("terminal.keys", ["right"]), ("terminal.keys", ["left"])]
# Fixed expected results, checked on each side alone (two binaries that do the same wrong thing still fail):
# step name -> (the "done" result, or None for a card step; exactly what it sent).
EXPECT = {
    "approve t20 wrong request": ("changed", []),
    "approve t20 deny": ("sent", [("terminal.keys", ["esc"])]),
    "approve t20 again (answered)": ("answered", []),
    "approve t1 allow": ("sent", [("terminal.keys", ["1"])]),
    "approve t9 allow (Enter on Yes, proceed)": ("sent", [("terminal.keys", ["enter"])]),
    "approve t10 deny": ("sent", [("terminal.keys", ["esc"])]),
    "answer t5 wrong request": ("changed", []),
    "answer t5 picks [1]": ("sent", [("terminal.keys", ["2"])]),
    "card t3 --all (walks the picker)": (None, WALK),
    "answer t3 two questions": ("sent", WALK + [("terminal.keys", [k]) for k in ("2", "1", "3", "tab", "1")]),
    "approve t20 allow, key held": ("unconfirmed", [("terminal.keys", ["1"])]),
    "approve t20 again while held": ("answered", []),
    "send t1 another kind": ("changed", []),
    "send t1 another log": ("changed", []),
    "send t1": ("sent", [("terminal.send", "hello — ✓")]),
    "send t1 --file (local)": ("sent", [("terminal.send", "look at '{work}/shot one.png'")]),
    "send box/t40 --file (remote)": ("sent", [("terminal.put", "shot one.png"), ("terminal.send", "look at '/remote/tmp/shot one.png'")]),
    "approve t4 (no identity)": ("no-identity", []),
    "send t4 (no identity)": ("no-identity", []),
}


def expectation_misses(steps, work):
    """The steps whose result or keys are not the fixed expected ones, and the expected steps that did not run."""
    misses, seen = [], set()
    for name, out, sent in steps:
        if name not in EXPECT:
            continue
        seen.add(name)
        result, want = EXPECT[name]
        want = [(m, v.replace("{work}", work) if isinstance(v, str) else v) for m, v in want]
        got = find(out, "done").get("result")
        if (result is not None and got != result) or brief(sent) != want:
            misses.append("%s: want %s %s, got %s %s" % (name, result, want, got, brief(sent)))
    misses += ["%s: did not run" % n for n in EXPECT if n not in seen]
    return misses


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True, help="first binary (Swift)")
    ap.add_argument("--b", required=True, help="second binary (Rust)")
    ap.add_argument("--report", help="write the full comparison here (JSON)")
    args = ap.parse_args()
    # The same path for both runs, so paths and cursors in the output can be compared as they are.
    work = os.path.join(tempfile.gettempdir(), "modfeed-parity-work")
    results = {}
    for side, binary in (("a", args.a), ("b", args.b)):
        r = Run(os.path.abspath(binary), work)
        try:
            results[side] = list(scenario(r))
        finally:
            r.close()
    problems = []
    names_a, names_b = [x[0] for x in results["a"]], [x[0] for x in results["b"]]
    if names_a != names_b:
        problems.append("the two runs made different steps: A %d, B %d; first difference at %s" % (
            len(names_a), len(names_b), next((i for i, (x, y) in enumerate(zip(names_a, names_b)) if x != y), min(len(names_a), len(names_b)))))
    for side in ("a", "b"):
        problems += ["%s: %s" % (side.upper(), m) for m in expectation_misses(results[side], work)]
    same, diffs, lines = 0, [], 0
    for (name, oa, sa), (_, ob, sb) in zip(results["a"], results["b"]):
        lines += len(oa["lines"])
        if oa["lines"] == ob["lines"] and oa["exit"] == ob["exit"] and sa == sb:
            same += 1
        else:
            diffs.append({"step": name, "a": oa["raw"], "b": ob["raw"], "a_sent": sa, "b_sent": sb})
    total = len(results["a"])
    print("modfeed parity: %d/%d commands identical (%d output lines from A), %d differ; %d fixed expectations, %d missed" % (
        same, total, lines, len(diffs), 2 * len(EXPECT), len(problems)))
    for p in problems:
        print("!!! " + p)
    for d in diffs:
        print("\n--- differs: %s" % d["step"])
        print("A:\n" + d["a"].rstrip())
        print("B:\n" + d["b"].rstrip())
        if d["a_sent"] != d["b_sent"]:
            print("A sent: %s\nB sent: %s" % (json.dumps(d["a_sent"]), json.dumps(d["b_sent"])))
    if args.report:
        os.makedirs(os.path.dirname(os.path.abspath(args.report)), exist_ok=True)
        json.dump({"identical": same, "total": total, "diffs": diffs, "expectations": 2 * len(EXPECT), "problems": problems},
                  open(args.report, "w"), indent=1, ensure_ascii=False)
    shutil.rmtree(work, ignore_errors=True)
    sys.exit(0 if not diffs and not problems and total > 0 else 1)


if __name__ == "__main__":
    main()
