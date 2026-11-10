#!/usr/bin/env python3
"""Builds the Flockbook demo in an isolated herdr-core (hc) core, from scripts/demo/content.py.

Run it through scripts/demo-hc.sh, which gives it a made-up HOME and HC_SESSION. Everything it writes is under HOME:
the fictional projects (HOME/code/<project>), the agents' logs (HOME/.claude/projects, HOME/.codex/sessions,
HOME/.flockbook-demo/logs), the stub's screens and scripts, and a few made-up Claude skills (the "/" list).

No model runs. Each agent is an hc terminal that runs the stub (stub.c) named claude, codex or pi, so hc detects that
agent. The stub holds the agent's log open, so Flockbook finds the conversation as the one log the agent has open.
  done      the stub shows a working screen, then idle: hc reports working -> idle on a terminal nobody has seen as done.
  working   a working screen.  idle: an idle screen.
  approval, question   the prompt; keys answer it (gen.interactive).
A Claude agent that is done or idle also answers a typed message (the stub's LINE block): the message goes into its log,
it works for a few seconds, then it replies with the next of REPLIES.

Spaces and tabs: the herdr-app document of the core (doc.set "herdr-app"), as the Herdr app writes it, so the phone page
and the app show the projects by name. Rebuild = remove every terminal of this core, then start them again.
--only NAME,...: rebuild only those agents (new terminals, so the phone page's push sees them turn blocked or done);
the others keep running and keep their place.
"""
import json, os, pwd, re, socket, subprocess, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import content  # noqa: E402
import gen  # noqa: E402

REPLY_WORK_SECONDS = 4
REPLIES = [
    "Done. I made that change and ran the tests again: **48 passed**, nothing else moved.\n\n"
    "Want me to open a pull request?",
    "Sure. I checked the other call sites too; two needed the same fix, and both are updated.",
    "Good idea. I added a note to the README and a test for the empty case.",
    "On it. That's in now, and the build is green.",
]
SKILLS = {
    "release-notes": "Write release notes for the changes since the last tag",
    "deploy-check": "Check that staging is healthy before a deploy",
    "triage": "Sort new issues by area and urgency",
    "changelog": "Add one line to the changelog for this branch",
    "screenshot-diff": "Compare UI screenshots before and after a change",
}


def osc7501(state, kind, app):
    return f"\x1b]7501;state={state}{':kind=' + kind if kind else ''}:app={app}\x07"


def with_status(draw):
    """A gen screen function whose screen starts with the hc status report (OSC 7501) for that screen.
    hc takes the status only from these reports. An idle screen is `done` when the agent worked to get there (its seeded
    status is not idle), and `idle` after a denied or declined prompt."""
    def screen(agent, cwd_label, state, *args, **kw):
        last = agent["turns"][-1][0] if agent.get("turns") else ""
        report = {"working": ("working", None), "approval": ("blocked", "permission"),
                  "question": ("blocked", "question")}.get(state)
        if report is None:
            report = ("idle", None) if agent["status"] == "idle" or last in ("deny", "askno") else ("done", None)
        return osc7501(*report, agent["kind"]) + draw(agent, cwd_label, state, *args, **kw)
    return screen


for _k, _f in (("claude", "claude_screen"), ("codex", "codex_screen"), ("pi", "pi_screen")):
    _wrapped = with_status(getattr(gen, _f))
    setattr(gen, _f, _wrapped)
    gen.SCREENS[_k] = _wrapped


def guard():
    """Refuse to run against a real home or a core that is not a demo one."""
    home = os.environ.get("HOME", "")
    session = os.environ.get("HC_SESSION", "")
    real = pwd.getpwuid(os.getuid()).pw_dir
    if not home or os.path.realpath(home) == os.path.realpath(real):
        raise SystemExit("hc_seed: HOME is the real home; run it through scripts/demo-hc.sh")
    if "demo" not in session or os.environ.get("HC_SOCKET"):
        raise SystemExit(f"hc_seed: HC_SESSION must name a demo core (got '{session}'), and HC_SOCKET must be unset")
    sock = core_socket()
    if not os.path.realpath(sock).startswith(os.path.realpath(home) + "/"):
        raise SystemExit(f"hc_seed: the core socket {sock} is not under HOME")


def core_socket():
    session = os.environ["HC_SESSION"]
    return os.path.join(os.environ["HOME"], ".local/state/herdr-core", session, f"{session}.sock")


def hc(*args, check=True):
    r = subprocess.run(["hc", args[0], "--json", *args[1:]], capture_output=True, text=True)
    out = r.stdout.strip()
    data = json.loads(out) if out.startswith(("{", "[")) else None
    if check and (r.returncode != 0 or (isinstance(data, dict) and "error" in data)):
        raise SystemExit(f"hc {' '.join(args)} failed: {r.stderr.strip() or out}")
    return data


def rpc(method, params):
    """One JSON-RPC call on the core's socket (as HcClient.call does)."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(10)
    s.connect(core_socket())
    s.sendall(json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode() + b"\n")
    buf = b""
    while True:
        chunk = s.recv(65536)
        if not chunk:
            raise SystemExit(f"{method}: the core closed the connection")
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            try:
                o = json.loads(line)
            except json.JSONDecodeError:
                continue
            if o.get("id") == 1:
                s.close()
                if "error" in o:
                    raise SystemExit(f"{method} failed: {o['error']}")
                return o.get("result")


def slug(s):
    return re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")


def reply_blocks(entry, label, state_dir):
    """LINE blocks for a Claude agent: up to len(REPLIES) typed messages, each answered after a short working spell."""
    agent, log, cwd, sid = entry["agent"], entry["log"], entry["cwd"], entry["sid"]
    d = f"{state_dir}/reply/{agent['name']}"
    turns = list(agent["turns"])
    base = len(gen.claude_log(agent, cwd, sid))
    blocks = []
    for k, reply in enumerate(REPLIES):
        asked = turns + [("u", "@@TEXT@@")]
        answered = asked + [("a", reply)]
        lines = gen.claude_log({**agent, "turns": answered}, cwd, sid)[base:]
        n_user = len(gen.claude_log({**agent, "turns": asked}, cwd, sid)) - base
        files = []
        for i, l in enumerate(lines):
            path = f"{d}/{k}-{i}.jsonl"
            gen.write(path, json.dumps({**l, "timestamp": "@@NOW@@"}, ensure_ascii=False) + "\n")
            files.append(path)
        shown = turns + [("u", "(message from Flockbook)")]
        working = f"{d}/{k}-working.ans"
        idle = f"{d}/{k}-idle.ans"
        gen.write(working, gen.claude_screen({**agent, "turns": shown, "verb": "Thinking", "elapsed": "2s"}, label, "working"))
        gen.write(idle, gen.claude_screen({**agent, "status": "done", "turns": shown + [("a", reply)]}, label, "idle"))
        here = "start" if k == 0 else f"r{k}"
        blocks.append([f"@ {here} LINE"] + [f"append {p} {log}" for p in files[:n_user]]
                      + [f"show {working}", f"sleep {REPLY_WORK_SECONDS}"] + [f"append {p} {log}" for p in files[n_user:]]
                      + [f"show {idle}", f"state r{k + 1}"])
        # The log grows as the stub appends; the next reply follows it.
        base += len(lines)
        turns = shown + [("a", reply)]
    return blocks


def scripts(plan, state_dir):
    """The stub's first screen and script for every agent that gen.prepare did not give a prompt script."""
    for entry in plan:
        agent, status = entry["agent"], entry["agent"]["status"]
        if entry["script"]:
            continue
        cwd = entry["cwd"]
        label = "~/" + os.path.relpath(cwd, os.environ["HOME"])
        blocks = [["@ start INIT", f"hold {entry['log']}"]]
        if status == "done":
            working = f"{state_dir}/screens/{agent['name']}.working.ans"
            gen.write(working, gen.SCREENS[agent["kind"]](agent, label, "working"))
            blocks[0] += ["sleep 3", f"show {entry['screen']}"]
            entry["screen"] = working
        if agent["kind"] == "claude" and status in ("done", "idle"):
            blocks += reply_blocks(entry, label, state_dir)
        path = f"{state_dir}/scripts/{agent['name']}.txt"
        gen.write(path, "\n".join("\n".join(b) for b in blocks) + "\n")
        entry["script"] = path


def skills(home):
    for name, about in SKILLS.items():
        gen.write(f"{home}/.claude/skills/{name}/SKILL.md", f"---\nname: {name}\ndescription: {about}\n---\n\n{about}.\n")


def main():
    guard()
    home = os.environ["HOME"]
    argv = sys.argv[1:]
    only = None
    if "--only" in argv:
        i = argv.index("--only")
        only = set(argv[i + 1].split(","))
        del argv[i:i + 2]
    bin_dir = argv[0] if argv else f"{home}/.flockbook-demo/bin"
    root, state_dir = f"{home}/code", f"{home}/.flockbook-demo"
    plan = gen.prepare(root, state_dir)
    scripts(plan, state_dir)
    skills(home)
    print(f"wrote {len(plan)} logs, screens and scripts under {state_dir}")

    if only is not None and not only <= {e["agent"]["name"] for e in plan}:
        raise SystemExit(f"--only: no such agent: {', '.join(sorted(only - {e['agent']['name'] for e in plan}))}")
    old = hc("ls", "--all") or []
    keep = {}
    for t in old if isinstance(old, list) else old.get("terminals", []):
        if only is not None and t.get("name") not in only and t.get("state") == "live":
            keep[t["name"]] = t["address"]
            continue
        if t.get("state") == "live":
            hc("kill", t["address"], check=False)
        hc("rm", t["address"], check=False)

    spaces, by_tab = [], {}
    for entry in plan:
        ws, tab, agent = entry["ws"], entry["tab"], entry["agent"]
        sp, tb = f"sp-{ws['dir']}", f"tb-{ws['dir']}-{slug(tab['label'])}"
        if agent["name"] in keep:
            addr = keep[agent["name"]]
        else:
            addr = hc("run", "--bg", "--no-parent", "--name", agent["name"], "--cwd", entry["cwd"],
                      "--label", f"herdr.space={sp}", "--label", f"herdr.tab={tb}",
                      "--", f"{bin_dir}/{agent['kind']}", entry["screen"], entry["script"])["address"]
        if not spaces or spaces[-1]["id"] != sp:
            spaces.append({"id": sp, "name": ws["label"], "folder": entry["cwd"], "tabs": []})
        if tb not in by_tab:
            by_tab[tb] = {"id": tb, "name": tab["label"], "root": {"id": f"{tb}-root", "type": "row", "children": []}}
            spaces[-1]["tabs"].append(by_tab[tb])
        by_tab[tb]["root"]["children"].append({"id": f"p-{agent['name']}", "type": "block", "content": {"terminal": addr}})
    version = rpc("doc.get", {"name": "herdr-app"})["version"]
    rpc("doc.set", {"name": "herdr-app", "value": {"spaces": spaces}, "version": version})
    print(f"started {len(plan) - len(keep)} agents ({len(keep)} kept) in {len(spaces)} spaces, {len(by_tab)} tabs")

    time.sleep(6)  # the stubs draw twice (1.5 s), done ones report done after 3 s
    terms = hc("ls")
    terms = terms if isinstance(terms, list) else terms.get("terminals", [])
    by = {}
    for t in terms:
        st = t.get("status") or "?"   # ls gives the state word; get gives the record
        st = st.get("state", "?") if isinstance(st, dict) else st
        by[st] = by.get(st, 0) + 1
    print("status: " + " ".join(f"{k}={v}" for k, v in sorted(by.items())))


if __name__ == "__main__":
    main()
