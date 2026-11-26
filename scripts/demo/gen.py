#!/usr/bin/env python3
"""Builds the Flockbook demo herdr session from scripts/demo/content.py.

No model runs. Each pane runs a stub (stub.c) named claude, codex or pi, so herdr detects that agent.
The stub prints a scripted screen; herdr reads the agent status from that screen (working spinner,
question picker, permission prompt). herdr shows "done" for a pane that went working -> idle while not focused.
A pane seeded as done runs the stub under its own name (not detected) and the script reports working, then idle.
The conversation comes from a log file this script writes:
  claude  ~/.claude/projects/-flockbook-demo/<id>.jsonl         (herdr keeps a Claude session as an id)
  codex   ~/.codex/sessions/2026/10/10/rollout-...-<id>.jsonl    (same for Codex)
  pi      <root>/.flockbook-demo/logs/<agent>.jsonl              (herdr keeps a pi session as a path)
Ids are fixed per agent, so a rerun overwrites the same files. Nothing is deleted.

Rebuild = create the new workspaces, then close every workspace that existed before the run.
"""
import argparse, base64, datetime as dt, json, os, subprocess, sys, textwrap, time, uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import content  # noqa: E402

W = 100  # screen width; narrower than the pane, so the app's terminal sheet does not wrap it
NS = uuid.UUID("6f1e9a52-3c1d-4b8e-9a8e-0d1f4c2b7a10")
PICTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pictures")  # drawn by pictures.mjs


def picture(name):
    """A Claude image block with scripts/demo/pictures/<name>."""
    with open(os.path.join(PICTURES, name), "rb") as f:
        data = base64.b64encode(f.read()).decode()
    mime = "image/jpeg" if name.endswith(".jpg") else "image/png"
    return {"type": "image", "source": {"type": "base64", "media_type": mime, "data": data}}


# ---------------------------------------------------------------- herdr

class Herdr:
    def __init__(self, session):
        self.session = session
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}

    def __call__(self, *args, check=True):
        r = subprocess.run(["herdr", "--session", self.session, *args], env=self.env, capture_output=True, text=True)
        out = r.stdout.strip()
        data = None
        if out:
            try:
                data = json.loads(out.splitlines()[-1])
            except json.JSONDecodeError:
                data = None
        if check and (r.returncode != 0 or (isinstance(data, dict) and "error" in data)):
            raise SystemExit(f"herdr {' '.join(args)} failed: {r.stderr.strip() or out}")
        return (data or {}).get("result", {})

    def socket(self):
        return os.path.expanduser(f"~/.config/herdr/sessions/{self.session}/herdr.sock")

    def ensure_server(self):
        try:
            self("workspace", "list")
            return
        except SystemExit:
            pass
        print(f"starting headless herdr server for session {self.session}")
        subprocess.Popen(["herdr", "--session", self.session, "server"], env=self.env,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        for _ in range(50):
            time.sleep(0.2)
            try:
                self("workspace", "list")
                return
            except SystemExit:
                pass
        raise SystemExit("herdr server did not start")

    def pane(self, pane_id):
        return self("pane", "get", pane_id).get("pane", {})


def find_pane_id(result):
    """Pane id in a create/split result (root_pane.pane_id or pane.pane_id)."""
    for key in ("root_pane", "pane"):
        if isinstance(result.get(key), dict) and "pane_id" in result[key]:
            return result[key]["pane_id"]
    raise SystemExit(f"no pane id in {result}")


# ---------------------------------------------------------------- times

def iso(t):
    return t.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"


def turn_times(agent, n):
    """n timestamps from `start` minutes ago to `end` minutes ago, evenly spaced."""
    now = dt.datetime.now(dt.timezone.utc)
    start, end = agent.get("start", 60), agent.get("end", 1)
    step = (start - end) / max(n - 1, 1)
    return [now - dt.timedelta(minutes=start - k * step) for k in range(n)]


# ---------------------------------------------------------------- logs

def claude_log(agent, cwd, sid):
    turns = agent["turns"]
    times = turn_times(agent, len(turns))
    lines, prev, n = [], None, 0

    def line(kind, message, ts, extra=None):
        nonlocal prev, n
        n += 1
        u = str(uuid.uuid5(NS, f"{sid}-{n}"))
        o = {"parentUuid": prev, "isSidechain": False, "userType": "external", "cwd": cwd, "sessionId": sid,
             "version": "2.1.296", "gitBranch": agent.get("branch", "main"), "type": kind, "message": message,
             "uuid": u, "timestamp": iso(ts)}
        if extra:
            o.update(extra)
        lines.append(o)
        prev = u

    def assistant(blocks, ts):
        line("assistant", {"id": f"msg_{uuid.uuid5(NS, f'{sid}-m{n}').hex[:24]}", "type": "message", "role": "assistant",
                           "model": "claude-opus-5-5", "content": blocks, "stop_reason": None}, ts)

    def result(tid, text, ts, err=False, extra=None):
        line("user", {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tid, "content": text, "is_error": err}]},
             ts + dt.timedelta(seconds=4), extra)

    last_tid = None
    for k, (t, ts) in enumerate(zip(turns, times)):
        tid = f"toolu_{uuid.uuid5(NS, f'{sid}-t{k}').hex[:24]}"
        op = t[0]
        if op == "u" and len(t) > 2:   # with pasted pictures
            line("user", {"role": "user", "content": [{"type": "text", "text": t[1]}] + [picture(n) for n in t[2]]}, ts)
        elif op == "u":
            line("user", {"role": "user", "content": t[1]}, ts)
        elif op == "deny":   # Esc on the permission prompt for the tool call just before
            result(last_tid, REJECTED, ts, True, extra={"toolUseResult": "User rejected tool use"})
            line("user", {"role": "user", "content": [{"type": "text", "text": "[Request interrupted by user for tool use]"}]},
                 ts + dt.timedelta(seconds=4))
        elif op == "askno":  # Esc on the question picker
            assistant([{"type": "tool_use", "id": tid, "name": "AskUserQuestion", "input": {"questions": t[1]}}], ts)
            result(tid, REJECTED, ts, True, extra={"toolUseResult": "User rejected tool use"})
        elif op == "a":
            assistant([{"type": "text", "text": t[1]}], ts)
        elif op == "ask":
            questions, answers = t[1], t[2]
            assistant([{"type": "tool_use", "id": tid, "name": "AskUserQuestion", "input": {"questions": questions}}], ts)
            text = "User has answered your questions: " + ", ".join(f'"{q}"="{a}"' for q, a in answers.items()) + \
                   ". You can now continue with the user's answers in mind."
            result(tid, text, ts, extra={"toolUseResult": {"questions": questions, "answers": answers}})
        else:
            name, inp, out, err = claude_tool(t, cwd)
            assistant([{"type": "tool_use", "id": tid, "name": name, "input": inp}], ts)
            if out is not None:
                result(tid, out, ts, err)
        last_tid = tid
    return lines


REJECTED = ("The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, "
            "the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.")


def claude_tool(t, cwd):
    op = t[0]
    absp = lambda p: p if p.startswith("/") else f"{cwd}/{p}"
    if op == "bash":
        cmd, out = t[1], t[2] if len(t) > 2 else None
        return "Bash", {"command": cmd, "description": t[4] if len(t) > 4 else describe(cmd)}, out, len(t) > 3 and t[3]
    if op == "readimg":
        return "Read", {"file_path": absp(t[1])}, [picture(t[2])], False
    if op == "read":
        body = t[2]
        numbered = "\n".join(f"{i + 1:>6}\t{l}" for i, l in enumerate(body.splitlines()))
        return "Read", {"file_path": absp(t[1])}, numbered, False
    if op == "edit":
        return "Edit", {"file_path": absp(t[1]), "old_string": t[2], "new_string": t[3]}, \
            f"The file {absp(t[1])} has been updated.", False
    if op == "write":
        return "Write", {"file_path": absp(t[1]), "content": t[2]}, f"File created successfully at: {absp(t[1])}", False
    if op == "grep":
        return "Grep", {"pattern": t[1], "output_mode": "content", "-n": True}, t[2], False
    if op == "glob":
        return "Glob", {"pattern": t[1]}, t[2], False
    if op == "task":
        return "Agent", {"description": t[1], "prompt": t[2], "subagent_type": "Explore"}, t[3], False
    if op == "todo":
        todos = [{"content": c, "status": s, "activeForm": c} for c, s in t[1]]
        return "TodoWrite", {"todos": todos}, "Todos have been modified successfully.", False
    if op == "fetch":
        return "WebFetch", {"url": t[1], "prompt": t[2]}, t[3], False
    if op == "tool":   # any other tool: ("tool", name, input, output)
        return t[1], t[2], t[3], False
    raise SystemExit(f"unknown turn {op}")


def describe(cmd):
    first = cmd.split()[0] if cmd.split() else "command"
    return {"pytest": "Run the test suite", "git": "Check git state", "npm": "Run npm script", "cargo": "Run cargo",
            "make": "Run make target", "rg": "Search the code"}.get(first, f"Run {first}")


def codex_log(agent, cwd, sid):
    turns = agent["turns"]
    times = turn_times(agent, len(turns))
    out = [{"timestamp": iso(times[0]), "type": "session_meta",
            "payload": {"id": sid, "timestamp": iso(times[0]), "cwd": cwd, "originator": "codex_cli_rs",
                        "cli_version": "0.130.0", "instructions": None}}]

    def item(payload, ts):
        out.append({"timestamp": iso(ts), "type": "response_item", "payload": payload})

    for k, (t, ts) in enumerate(zip(turns, times)):
        cid = f"call_{uuid.uuid5(NS, f'{sid}-c{k}').hex[:22]}"
        op = t[0]
        if op == "u":
            item({"type": "message", "role": "user", "content": [{"type": "input_text", "text": t[1]}]}, ts)
        elif op == "a":
            last = k == len(turns) - 1 or turns[k + 1][0] == "u"
            item({"type": "message", "role": "assistant", "phase": "final_answer" if last else "commentary",
                  "content": [{"type": "output_text", "text": t[1]}]}, ts)
        elif op in ("edit", "write"):
            if op == "edit":
                minus = "\n".join("-" + l for l in t[2].splitlines())
                plus = "\n".join("+" + l for l in t[3].splitlines())
                patch = f"*** Begin Patch\n*** Update File: {t[1]}\n@@\n{minus}\n{plus}\n*** End Patch"
                res = f"Success. Updated the following files:\nM {t[1]}"
            else:
                body = "\n".join("+" + l for l in t[2].splitlines())
                patch = f"*** Begin Patch\n*** Add File: {t[1]}\n{body}\n*** End Patch"
                res = f"Success. Updated the following files:\nA {t[1]}"
            item({"type": "custom_tool_call", "name": "apply_patch", "call_id": cid, "input": patch}, ts)
            item({"type": "custom_tool_call_output", "call_id": cid, "output": res}, ts + dt.timedelta(seconds=2))
        else:
            cmd, res, err = shell_of(t)
            item({"type": "function_call", "name": "shell", "call_id": cid,
                  "arguments": json.dumps({"command": ["bash", "-lc", cmd], "workdir": cwd, "timeout_ms": 120000})}, ts)
            if res is not None:
                item({"type": "function_call_output", "call_id": cid,
                      "output": json.dumps({"output": res, "metadata": {"exit_code": 1 if err else 0, "duration_seconds": 1.2}})},
                     ts + dt.timedelta(seconds=3))
    return out


def shell_of(t):
    """A turn as a shell command (Codex and pi run everything through a shell)."""
    op = t[0]
    if op == "bash":
        return t[1], (t[2] if len(t) > 2 else None), (len(t) > 3 and t[3])
    if op == "read":
        return f"sed -n '1,200p' {t[1]}", t[2], False
    if op == "grep":
        return f"rg -n \"{t[1]}\"", t[2], False
    if op == "glob":
        return f"rg --files -g '{t[1]}'", t[2], False
    raise SystemExit(f"turn {op} is Claude only")


def pi_log(agent, cwd, sid):
    turns = agent["turns"]
    times = turn_times(agent, len(turns))
    out, prev, n = [], None, 0

    def entry(o, ts):
        nonlocal prev, n
        n += 1
        eid = f"{uuid.uuid5(NS, f'{sid}-{n}').hex[:8]}"
        o = {**o, "id": eid, "parentId": prev, "timestamp": iso(ts)}
        out.append(o)
        prev = eid

    out.append({"type": "session", "version": 3, "id": sid, "timestamp": iso(times[0]), "cwd": cwd})
    prev = None
    entry({"type": "model_change", "provider": "deepseek", "modelId": "deepseek-v4"}, times[0])
    usage = {"input": 1200, "output": 300, "cacheRead": 0, "cacheWrite": 0, "reasoning": 0, "totalTokens": 1500,
             "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}}

    def assistant(blocks, ts, stop):
        entry({"type": "message", "message": {"role": "assistant", "content": blocks, "api": "openai-completions",
                                               "provider": "deepseek", "model": "deepseek-v4", "usage": usage,
                                               "stopReason": stop, "timestamp": int(ts.timestamp() * 1000)}}, ts)

    for k, (t, ts) in enumerate(zip(turns, times)):
        cid = f"call_{uuid.uuid5(NS, f'{sid}-p{k}').hex[:16]}"
        op = t[0]
        ms = int(ts.timestamp() * 1000)
        if op == "u":
            entry({"type": "message", "message": {"role": "user", "content": [{"type": "text", "text": t[1]}], "timestamp": ms}}, ts)
            continue
        if op == "a":
            assistant([{"type": "text", "text": t[1]}], ts, "stop")
            continue
        if op == "read":
            name, args, res, err = "read", {"path": t[1]}, t[2], False
        elif op == "edit":
            name, args, res, err = "edit", {"path": t[1], "edits": [{"oldText": t[2], "newText": t[3]}]}, \
                f"Successfully replaced text in {t[1]}.", False
        elif op == "write":
            name, args, res, err = "write", {"path": t[1], "content": t[2]}, f"Wrote {t[1]}", False
        else:
            cmd, res, err = shell_of(t)
            name, args = "bash", {"command": cmd}
        assistant([{"type": "toolCall", "id": cid, "name": name, "arguments": args}], ts, "toolUse")
        if res is not None:
            rts = ts + dt.timedelta(seconds=3)
            entry({"type": "message", "message": {"role": "toolResult", "toolCallId": cid, "toolName": name,
                                                   "content": [{"type": "text", "text": res}], "details": {},
                                                   "isError": bool(err), "timestamp": int(rts.timestamp() * 1000)}}, rts)
    return out


# ---------------------------------------------------------------- screens

DIM, BOLD, RESET, ORANGE, GREEN, RED, GRAY = "\x1b[2m", "\x1b[1m", "\x1b[0m", "\x1b[38;5;173m", "\x1b[32m", "\x1b[31m", "\x1b[90m"
RULE = "─" * W


def wrap(text, width, first="", rest=""):
    out = []
    for para in text.splitlines() or [""]:
        if not para.strip():
            out.append("")
            continue
        out += textwrap.wrap(para, width, initial_indent=first if not out else rest, subsequent_indent=rest,
                             replace_whitespace=False, drop_whitespace=True) or [first]
    return out


def strip_md(text):
    lines, in_code = [], False
    for l in text.splitlines():
        if l.startswith("```"):
            in_code = not in_code
            continue
        lines.append(l.replace("**", "").replace("`", ""))
    return "\n".join(lines)


def shorten(s, n):
    s = " ".join(s.split())
    return s if len(s) <= n else s[: n - 1] + "…"


def claude_body(agent, cwd_label):
    out = [f"{ORANGE} ▐▛███▜▌{RESET}   {BOLD}Claude Code{RESET} v2.1.296",
           f"{ORANGE}▝▜█████▛▘{RESET}  Opus 5.5 · Claude Max",
           f"{ORANGE}  ▘▘ ▝▝{RESET}    {cwd_label}", ""]
    for t in agent["turns"]:
        op = t[0]
        if op == "u":
            text = t[1].split("<pasted_content")[0].strip() or "[Pasted text]"
            if len(t) > 2:
                text += "".join(f" [Image #{i + 1}]" for i in range(len(t[2])))
            out += [""] + [f"{GRAY}{l}{RESET}" for l in wrap(text, W - 4, "❯ ", "  ")[:6]] + [""]
        elif op == "a":
            out += wrap(strip_md(t[1]), W - 4, "⏺ ", "  ")[:10] + [""]
        elif op == "ask":
            out += ["⏺ User answered Claude's questions:"]
            out += [f"  ⎿  · {shorten(q, 70)} → {a}" for q, a in t[2].items()] + [""]
        elif op == "deny":
            if out and out[-1] == "":
                out.pop()
            if out and "Running…" in out[-1]:
                out.pop()
            out += [f"  ⎿  {RED}No (tell Claude what to do differently){RESET}", ""]
        elif op == "askno":
            out += ["⏺ User declined to answer questions", ""]
        else:
            name, inp, res, err = claude_tool(t, "")
            arg = {"Bash": inp.get("command", ""), "Read": t[1], "Edit": t[1], "Write": t[1],
                   "Grep": f'pattern: "{t[1]}"', "Glob": f'pattern: "{t[1]}"', "Agent": t[1] if op == "task" else "",
                   "TodoWrite": "", "WebFetch": t[1] if op == "fetch" else "",
                   "Artifact": inp.get("title", "") if op == "tool" else ""}.get(name, "")
            shown = {"Edit": "Update", "Grep": "Search", "Agent": "Explore", "TodoWrite": "Update Todos",
                     "WebFetch": "Fetch"}.get(name, name)
            dot = f"{RED}⏺{RESET}" if err else f"{GREEN}⏺{RESET}"
            out.append(f"{dot} {BOLD}{shown}{RESET}({shorten(arg, W - 20)})" if arg else f"{dot} {BOLD}{shown}{RESET}")
            if res is None:
                out.append(f"  ⎿  {DIM}Running…{RESET}")
            elif op == "readimg":
                out.append(f"  ⎿  Read image ({os.path.getsize(os.path.join(PICTURES, t[2])) / 1024:.1f}KB)")
            elif name == "Read":
                out.append(f"  ⎿  Read {len(t[2].splitlines())} lines")
            elif name == "Edit":
                out.append(f"  ⎿  Updated {t[1]} with {len(t[3].splitlines())} additions and {len(t[2].splitlines())} removals")
            elif name in ("Write",):
                out.append(f"  ⎿  Wrote {len(t[2].splitlines())} lines to {t[1]}")
            elif name == "TodoWrite":
                out += [f"  ⎿  {'☒' if s == 'completed' else '☐'} {c}" for c, s in t[1]]
            else:
                rl = (res or "").splitlines() or [""]
                out.append(f"  ⎿  {shorten(rl[0], W - 8)}")
                out += [f"     {shorten(l, W - 8)}" for l in rl[1:3]]
                if len(rl) > 3:
                    out.append(f"     {DIM}… +{len(rl) - 3} lines (ctrl+o to expand){RESET}")
            out.append("")
    return out


def claude_screen(agent, cwd_label, state, cursor=0):
    body = claude_body(agent, cwd_label)
    title = agent["title"]
    if state == "working" and agent.get("queued"):   # messages typed while it works (Flockbook shows them)
        foot = [f"❯ {q}" for q in agent["queued"]] + [f"  {DIM}ctrl+enter to send now{RESET}",
                f"{ORANGE}✻{RESET} {agent.get('verb', 'Working')}… ({agent.get('elapsed', '1m 12s')} · {DIM}esc to interrupt{RESET})",
                GRAY + RULE + RESET, f"❯ {DIM}Press up to edit queued messages{RESET}", GRAY + RULE + RESET,
                f"  {DIM}⏵⏵ accept edits on (shift+tab to cycle){RESET}"]
        osc = f"⠂ {title}"
    elif state == "working":
        foot = [f"{ORANGE}✻{RESET} {agent.get('verb', 'Working')}… ({agent.get('elapsed', '1m 12s')} · {DIM}esc to interrupt{RESET})",
                "", GRAY + RULE + RESET, "❯ ", GRAY + RULE + RESET, f"  {DIM}⏵⏵ accept edits on (shift+tab to cycle){RESET}"]
        osc = f"⠂ {title}"
    elif state == "approval":
        cmd = agent["turns"][-1][1]
        # Claude Code 2.1.296 layout: description, then the command between dashed rules (the app matches the log's call
        # with the command it reads there; the older layout puts the description under the command and never matches).
        foot = [RULE, f" {BOLD}Bash command{RESET}", f" {DIM}{agent.get('approval_why', describe(cmd))}{RESET}",
                GRAY + "╌" * W + RESET, f" {cmd}", GRAY + "╌" * W + RESET,
                " Do you want to proceed?", f" {ORANGE}❯{RESET} 1. Yes",
                f"   2. Yes, and don't ask again for {cmd.split()[0]} commands in {cwd_label}", "   3. No, and tell Claude what to do differently (esc)", "", " Esc to cancel · Tab to amend"]
        osc = f"✳ {title}"
    elif state == "question":
        q = agent["question"]
        opts = []
        for i, o in enumerate(q["options"], 1):
            mark = f"{ORANGE}❯{RESET} " if i == cursor + 1 else "  "
            opts.append(f"{mark}{i}. {o['label']}")
            opts += wrap(o["description"], W - 8, "     ", "     ")
        n = len(q["options"])
        foot = [RULE, f" ☐ {q['header']}", "", q["question"], ""] + opts + \
               [f"  {n + 1}. Type something.", RULE, f"  {n + 2}. Chat about this", "",
                "Enter to select · ↑/↓ to navigate · Esc to cancel"]
        osc = f"✳ {title}"
    else:
        foot = [GRAY + RULE + RESET, "❯ ", GRAY + RULE + RESET, f"  {DIM}? for shortcuts{RESET}"]
        osc = f"✳ {title}"
    return compose(osc, body, foot)


def codex_screen(agent, cwd_label, state):
    out = ["╭──────────────────────────────────────────────╮",
           f"│ >_ {BOLD}OpenAI Codex{RESET} (v0.130.0)                     │",
           "│                                              │",
           f"│ model:     gpt-5.5 high                      │",
           f"│ directory: {cwd_label:<34}│",
           "╰──────────────────────────────────────────────╯", ""]
    for t in agent["turns"]:
        op = t[0]
        if op == "u":
            out += [""] + [f"{BOLD}{l}{RESET}" for l in wrap(t[1], W - 4, "› ", "  ")[:6]] + [""]
        elif op == "a":
            out += wrap(strip_md(t[1]), W - 4, "• ", "  ")[:10] + [""]
        elif op in ("edit", "write"):
            add = len((t[3] if op == "edit" else t[2]).splitlines())
            rem = len(t[2].splitlines()) if op == "edit" else 0
            out += [f"• {'Edited' if op == 'edit' else 'Added'} {t[1]} ({GREEN}+{add}{RESET} {RED}-{rem}{RESET})", ""]
        else:
            cmd, res, err = shell_of(t)
            if op == "read":
                out += ["• Explored", f"  └ Read {t[1]}", ""]
                continue
            out.append(f"• {'Running' if res is None else 'Ran'} {shorten(cmd, W - 12)}")
            rl = (res or "").splitlines()
            out += [f"  └ {shorten(l, W - 8)}" if i == 0 else f"    {shorten(l, W - 8)}" for i, l in enumerate(rl[:3])]
            if len(rl) > 3:
                out.append(f"    {DIM}… +{len(rl) - 3} lines{RESET}")
            out.append("")
    title = agent["title"]
    if state == "working" and agent.get("queued"):   # follow-ups typed while it works (Flockbook shows them)
        foot = [f"• Working ({agent.get('elapsed', '48s')} • {DIM}esc to interrupt{RESET})", "", "• Queued follow-up inputs"]
        foot += [f"  ↳ {q}" for q in agent["queued"]]
        foot += [f"    {DIM}⇧← edit last queued message{RESET}", "", f"› {DIM}Ask Codex to do anything{RESET}", "",
                 f"  {DIM}gpt-5.5 high · 71% context left{RESET}"]
        osc = f"⠋ {title}"
    elif state == "working":
        foot = [f"• Working ({agent.get('elapsed', '48s')} • {DIM}esc to interrupt{RESET})", "", "› ", "",
                f"  {DIM}gpt-5.5 high · 71% context left{RESET}"]
        osc = f"⠋ {title}"
    else:
        foot = ["", f"› {DIM}Ask Codex to do anything{RESET}", "", f"  {DIM}gpt-5.5 high · 64% context left · ? for shortcuts{RESET}"]
        osc = title
    return compose(osc, out, foot)


def pi_screen(agent, cwd_label, state):
    out = [f"{BOLD}pi{RESET} v0.70.2", f"{DIM}{cwd_label} · deepseek-v4{RESET}", ""]
    for t in agent["turns"]:
        op = t[0]
        if op == "u":
            out += [""] + [f"{BOLD}{l}{RESET}" for l in wrap(t[1], W - 4, "", "")[:6]] + [""]
        elif op == "a":
            out += wrap(strip_md(t[1]), W - 2)[:10] + [""]
        else:
            if op == "read":
                out += [f"{BOLD}read{RESET} {t[1]}", ""]
            elif op in ("edit", "write"):
                out += [f"{BOLD}{op}{RESET} {t[1]}", ""]
            else:
                cmd, res, err = shell_of(t)
                out.append(f"{BOLD}${RESET} {shorten(cmd, W - 4)}")
                out += [f"{DIM}{shorten(l, W - 4)}{RESET}" for l in (res or "").splitlines()[:3]]
                out.append("")
    title = agent["title"]
    if state == "working":
        foot = ["⠋ Working...", "", GRAY + RULE + RESET, "", GRAY + RULE + RESET, f"{DIM}{cwd_label}  ↑1.2k ↓300  deepseek-v4{RESET}"]
        osc = title
    else:
        foot = [GRAY + RULE + RESET, "", GRAY + RULE + RESET, f"{DIM}{cwd_label}  ↑14k ↓3.1k  deepseek-v4{RESET}"]
        osc = title
    return compose(osc, out, foot)


def compose(osc, body, foot, rows=40):
    room = rows - len(foot) - 1
    body = body[-room:] if len(body) > room else body
    lines = body + [""] * (room - len(body)) + foot
    return f"\x1b]0;{osc}\x07\x1b[2J\x1b[3J\x1b[H" + "\r\n".join(lines)


SCREENS = {"claude": claude_screen, "codex": codex_screen, "pi": pi_screen}
LOGS = {"claude": claude_log, "codex": codex_log, "pi": pi_log}


# ---------------------------------------------------------------- main

def log_path(kind, name, sid, state_dir):
    if kind == "claude":
        return os.path.expanduser(f"~/.claude/projects/-flockbook-demo/{sid}.jsonl")
    if kind == "codex":
        return os.path.expanduser(f"~/.codex/sessions/2026/10/10/rollout-2026-10-10T08-00-00-{sid}.jsonl")
    return f"{state_dir}/logs/{name}.jsonl"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def prepare(root, state_dir):
    """Writes project folders, logs and screens. Returns the plan: one entry per pane."""
    plan = []
    for ws in content.WORKSPACES:
        cwd = f"{root}/{ws['dir']}"
        os.makedirs(cwd, exist_ok=True)
        readme = f"{cwd}/README.md"
        if not os.path.exists(readme):
            write(readme, f"# {ws['label']}\n\n{ws['about']}\n\nFictional project for the Flockbook demo.\n")
        for tab in ws["tabs"]:
            for agent in tab["panes"]:
                kind, name = agent["kind"], agent["name"]
                sid = str(uuid.uuid5(NS, name))
                path = log_path(kind, name, sid, state_dir)
                lines = LOGS[kind](agent, cwd, sid)
                write(path, "".join(json.dumps(l, ensure_ascii=False) + "\n" for l in lines))
                status = agent["status"]
                draw = SCREENS[kind]
                label = f"~/{os.path.relpath(cwd, os.path.expanduser('~'))}" if cwd.startswith(os.path.expanduser("~")) else cwd
                screen = f"{state_dir}/screens/{name}.ans"
                write(screen, draw(agent, label, "idle" if status == "done" else status))
                script = interactive(agent, cwd, sid, label, path, lines, state_dir) \
                    if status in ("approval", "question") else None
                plan.append({"ws": ws, "tab": tab, "agent": agent, "sid": sid, "log": path, "screen": screen, "cwd": cwd,
                             "script": script})
    return plan


# After an answer the pane must end "done", as a real agent does when it finishes unseen. herdr reads working and
# idle from the screen, and working -> idle on a pane that is not focused gives done, except for the first turn after
# a blocked prompt (that one ends idle). So the answer first shows a short idle screen, then working, then idle.
BLIP_SECONDS, WORK_SECONDS = 0.5, 4


def interactive(agent, cwd, sid, label, log, base, state_dir):
    """Writes the stub SCRIPT that answers the prompt: screens per step and the log lines to append.
    Approval: 1, 2 or Enter = allow (tool result, working, then the follow-up and done); Esc or 3 = deny (idle).
    Question: a digit or Enter on the highlighted option = pick (answer logged, working, follow-up, done);
    Up/Down move the highlight; Esc = decline."""
    if agent["kind"] != "claude":
        raise SystemExit(f"{agent['name']}: only Claude panes answer prompts")
    name, d = agent["name"], f"{state_dir}/interactive/{agent['name']}"
    blocks, n_files = [], [0]

    def screen(turns, state, **kw):
        n_files[0] += 1
        path = f"{d}/{n_files[0]:02d}-{state}.ans"
        write(path, claude_screen({**agent, **kw, "turns": turns}, label, state, **({"cursor": kw["cursor"]} if "cursor" in kw else {})))
        return path

    def tail(turns):
        """Log lines that `turns` adds after the seeded log, one file per line, with the time left to the stub."""
        lines = claude_log({**agent, "turns": turns}, cwd, sid)[len(base):]
        out = []
        for l in lines:
            n_files[0] += 1
            path = f"{d}/{n_files[0]:02d}-line.jsonl"
            write(path, json.dumps({**l, "timestamp": "@@NOW@@"}, ensure_ascii=False) + "\n")
            out.append(path)
        return out

    def finish(head, done_turns, verb):
        """Block body for an answer: append the answer's lines, show idle briefly, then working, wait, then append
        the follow-up and show the idle screen."""
        lines = tail(done_turns)
        k = len(claude_log({**agent, "turns": head}, cwd, sid)) - len(base)
        first, rest = lines[:k], lines[k:]
        return ([f"append {p} {log}" for p in first] + [f"show {screen(head, 'idle')}", f"sleep {BLIP_SECONDS}",
                f"show {screen(head, 'working', verb=verb, elapsed='3s')}", f"sleep {WORK_SECONDS}"] + [f"append {p} {log}" for p in rest]
                + [f"show {screen(done_turns, 'idle')}", "state done"])

    turns = agent["turns"]
    if agent["status"] == "approval":
        cmd = turns[-1][1]
        ran = turns[:-1] + [("bash", cmd, agent["allow"]["output"], agent["allow"].get("error", False))]
        allow = finish(ran, ran + agent["allow"]["then"], agent["allow"].get("verb", "Thinking"))
        denied = turns + [("deny",)]
        deny = [f"append {p} {log}" for p in tail(denied)] + [f"show {screen(denied, 'idle')}", "state denied"]
        for key in ("1", "2", "ENTER"):
            blocks.append([f"@ start {key}"] + allow)
        for key in ("ESC", "3"):
            blocks.append([f"@ start {key}"] + deny)
    else:
        q = agent["question"]
        qs = [{"question": q["question"], "header": q["header"], "options": q["options"], "multiSelect": False}]
        opts = q["options"]
        states = ["start"] + [f"c{k}" for k in range(1, len(opts))]
        for k, st in enumerate(states):
            if k:
                blocks.append([f"@ {states[k - 1]} DOWN", f"show {screen(turns, 'question', cursor=k)}", f"state {st}"])
                blocks.append([f"@ {st} UP", f"show {screen(turns, 'question', cursor=k - 1)}", f"state {states[k - 1]}"])
        picks = []
        for k, o in enumerate(opts):
            head = turns + [("ask", qs, {q["question"]: o["label"]})]
            follow = agent.get("answers", {}).get(o["label"].replace(" (Recommended)", "")) or \
                agent.get("answer_default", "Got it: {label}. I'll start on that now.").format(label=o["label"].replace(" (Recommended)", ""))
            picks.append(finish(head, head + [("a", follow)], "Thinking"))
            blocks.append([f"@ {states[0]} {k + 1}"] + picks[k])
            for st in states[1:]:
                blocks.append([f"@ {st} {k + 1}"] + picks[k])
        for k, st in enumerate(states):
            blocks.append([f"@ {st} ENTER"] + picks[k])
        declined = turns + [("askno", qs)]
        decline = [f"append {p} {log}" for p in tail(declined)] + [f"show {screen(declined, 'idle')}", "state declined"]
        for st in states:
            blocks.append([f"@ {st} ESC"] + decline)
    # No session is reported for this pane: with a reported session, herdr ends a screen-detected working -> idle as
    # idle, not done. Flockbook finds the log as the one agent log the pane's process holds open.
    blocks.insert(0, ["@ start INIT", f"hold {log}"])
    path = f"{d}/script.txt"
    write(path, "\n".join("\n".join(b) for b in blocks) + "\n")
    return path


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--session", default="flockbook-demo")
    ap.add_argument("--root", default=os.path.expanduser("~/Demo"), help="parent folder of the fictional projects")
    ap.add_argument("--bin", required=True, help="folder with the stub installed as claude, codex and pi")
    ap.add_argument("--files-only", action="store_true", help="write logs and screens, do not touch herdr")
    a = ap.parse_args()
    if a.session == "default" or "demo" not in a.session:   # never seed a working session by mistake
        raise SystemExit(f"refusing to seed session '{a.session}': the name must contain 'demo'")
    root = os.path.abspath(a.root)
    state_dir = f"{root}/.flockbook-demo"
    plan = prepare(root, state_dir)
    print(f"wrote {len(plan)} logs and screens under {state_dir}")
    if a.files_only:
        return

    h = Herdr(a.session)
    h.ensure_server()
    old = [w["workspace_id"] for w in h("workspace", "list").get("workspaces", [])]

    panes, roots = [], {}
    for entry in plan:
        ws, tab, agent = entry["ws"], entry["tab"], entry["agent"]
        wkey, tkey = ws["label"], (ws["label"], tab["label"])
        if wkey not in roots:
            r = h("workspace", "create", "--label", ws["label"], "--cwd", entry["cwd"], "--no-focus")
            pid = find_pane_id(r)
            h("tab", "rename", r["tab"]["tab_id"], tab["label"])
            roots[wkey] = r["workspace"]["workspace_id"]
            roots[tkey] = pid
        elif tkey not in roots:
            r = h("tab", "create", "--workspace", roots[wkey], "--label", tab["label"], "--cwd", entry["cwd"], "--no-focus")
            pid = find_pane_id(r)
            roots[tkey] = pid
        else:
            r = h("pane", "split", roots[tkey], "--direction", agent.get("split", "right"), "--cwd", entry["cwd"], "--no-focus")
            pid = find_pane_id(r)
        stub = f"{a.bin}/{'demo-agent' if agent['status'] == 'done' else agent['kind']}"
        h("pane", "run", pid, "exec " + " ".join(sh_quote(x) for x in [stub, entry["screen"]] + ([entry["script"]] if entry["script"] else [])))
        panes.append((pid, entry))
    if old:
        for wid in old:
            h("workspace", "close", wid, check=False)

    for pid, entry in panes:
        agent, kind = entry["agent"], entry["agent"]["kind"]
        if agent["status"] == "done":
            h("pane", "report-agent", pid, "--source", "demo:seed", "--agent", kind, "--state", "working",
              "--seq", str(time.time_ns()))
        else:
            for _ in range(40):
                if h.pane(pid).get("agent") == kind:
                    break
                time.sleep(0.25)
            else:
                print(f"warning: herdr did not detect {kind} in {pid}")
            if not entry["script"]:
                report_session(h, pid, entry)
        h("agent", "rename", pid, agent["name"])

    focus = next(pid for pid, e in panes if e["agent"].get("focus") and not e["script"])  # a focused prompt pane ends idle
    h("agent", "focus", focus)
    # working -> idle while not focused = done. The session goes last: after it, herdr ignores state reports.
    # (herdr takes no pi session on a pane it did not detect, so done panes are claude or codex.)
    for pid, entry in panes:
        if entry["agent"]["status"] == "done":
            h("pane", "report-agent", pid, "--source", "demo:seed", "--agent", entry["agent"]["kind"], "--state", "idle",
              "--seq", str(time.time_ns()))
            report_session(h, pid, entry)
    time.sleep(6)  # screen detection settles
    summary(h)


def report_session(h, pid, entry):
    kind, seq = entry["agent"]["kind"], str(time.time_ns())
    ref = ["--agent-session-path", entry["log"]] if kind == "pi" else ["--agent-session-id", entry["sid"]]
    h("pane", "report-agent-session", pid, "--source", f"herdr:{kind}", "--agent", kind, "--seq", seq,
      *ref, "--session-start-source", "startup")


def sh_quote(s):
    return "'" + s.replace("'", "'\\''") + "'"


def summary(h):
    snap = h("api", "snapshot").get("snapshot", {})
    agents = snap.get("agents", [])
    tabs = {(p["workspace_id"], p["tab_id"]) for p in snap.get("panes", [])}
    ws = {p["workspace_id"] for p in snap.get("panes", [])}
    by_status, by_kind, missing = {}, {}, []
    for ag in agents:
        by_status[ag["agent_status"]] = by_status.get(ag["agent_status"], 0) + 1
        by_kind[ag["agent"]] = by_kind.get(ag["agent"], 0) + 1
        if not ag.get("agent_session"):
            missing.append(ag.get("name", ag["pane_id"]))
    print(f"workspaces={len(ws)} tabs={len(tabs)} panes={len(snap.get('panes', []))} agents={len(agents)}")
    print("status: " + " ".join(f"{k}={v}" for k, v in sorted(by_status.items())))
    print("kinds:  " + " ".join(f"{k}={v}" for k, v in sorted(by_kind.items())))
    if missing:
        print("no session reported (prompt panes; Flockbook finds the log by the open file): " + ", ".join(missing))
    print("note: do not focus prompt panes in a herdr TUI while recording (a seen pane ends idle, not done)")


if __name__ == "__main__":
    main()
