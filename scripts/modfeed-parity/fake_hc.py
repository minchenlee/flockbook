"""A fake herdr-core on a Unix socket for the modfeed parity check (no real hc is ever touched).

Plain-mode JSON-RPC, one request per connection, as the sidecars use it. State comes from a JSON file that the
driver rewrites between commands: {"terminals": [...], "screens": {addr: text}, "after_keys": {addr: text}}.
Every call is appended to calls.jsonl as {"method", "params"}. `terminal.keys` moves the screen to `after_keys[addr]`
when one is set (a prompt that goes away once answered).

Also in the state:
- "pickers": {addr: picker}: Claude Code's question picker (2.1.296, as herdr-core's `question/tests.rs` Sim draws it),
  driven by the keys: digits pick or toggle, left/right move between questions, tab moves on, the review screen's
  `1` submits. Single choice and multiSelect without "Other" only.
- "delay_keys": {addr: seconds}: `terminal.keys` answers that much later (a key hc held: the sidecar times out).
- `terminal.put` answers a path on the remote machine: "/remote/tmp/<name>".
"""
import json, os, socket, sys, threading, time

sock_path, state_path, calls_path = sys.argv[1], sys.argv[2], sys.argv[3]
lock = threading.Lock()


def load():
    with open(state_path) as f:
        return json.load(f)


def save(st):
    tmp = state_path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(st, f)
    os.replace(tmp, state_path)


ABOVE = "❯ Use AskUserQuestion\n" + "─" * 40


def picker_question(header, text, options, multi=False):
    """One question of a picker: options are (label, detail) pairs."""
    return {"header": header, "text": text, "options": [list(o) for o in options], "multi": multi}


def new_picker(questions):
    n = len(questions)
    return {"qs": questions, "at": 0, "focus": 0, "review": False, "done": False, "checks": [[] for _ in range(n)],
            "single": {}}


def _answered(p, k):
    return str(k) in p["single"] or bool(p["checks"][k])


def _bar(p):
    qs = p["qs"]
    if len(qs) == 1 and not qs[0]["multi"]:
        return " %s %s" % ("☒" if _answered(p, 0) else "☐", qs[0]["header"])
    tabs = ["%s %s" % ("☒" if _answered(p, k) else "☐", q["header"]) for k, q in enumerate(qs)]
    return "←  %s  ✔ Submit  →" % "  ".join(tabs)


def render(p):
    if p["done"]:
        return ABOVE + "\n⏺ User answered Claude's questions:\n❯ "
    if p["review"]:
        l = [ABOVE, _bar(p), "", "Review your answers", ""]
        for k, q in enumerate(p["qs"]):
            picked = [q["options"][i][0] for i in sorted(p["checks"][k])] if q["multi"] else \
                [q["options"][p["single"][str(k)]][0]] if str(k) in p["single"] else [""]
            l += [" ● " + q["text"], "   → " + ", ".join(picked)]
        l += ["", "Ready to submit your answers?", "", "❯ 1. Submit answers", "  2. Cancel"]
        return "\n".join(l)
    q = p["qs"][p["at"]]
    n = len(q["options"])
    mark = lambda i: "❯" if p["focus"] == i else " "
    l = [ABOVE, _bar(p), "", q["text"], ""]
    for i, (label, detail) in enumerate(q["options"]):
        box = ("[✔] " if i in p["checks"][p["at"]] else "[ ] ") if q["multi"] else ""
        l.append("%s %d. %s%s" % (mark(i), i + 1, box, label))
        if detail:
            l.append(("         " if q["multi"] else "     ") + detail)
    l.append("%s %d. %s%s" % (mark(n), n + 1, "[ ] " if q["multi"] else "", "Type something" if q["multi"] else "Type something."))
    if q["multi"]:
        l.append("%s    Submit" % mark(n + 1))
    l += ["─" * 60, "  %d. Chat about this" % (n + 2), ""]
    nav = "Tab/Arrow keys to navigate" if len(p["qs"]) > 1 else "↑/↓ to navigate"
    l.append("Enter to select · %s · Esc to cancel" % nav)
    return "\n".join(l)


def _next(p):
    p["focus"] = 0
    if p["at"] + 1 < len(p["qs"]):
        p["at"] += 1
    elif len(p["qs"]) > 1 or p["qs"][p["at"]]["multi"]:
        p["review"] = True
    else:
        p["done"] = True


def press(p, k):
    if p["done"]:
        return
    if p["review"]:
        if k == "1":
            p["review"], p["done"] = False, True
        elif k == "left":
            p["review"] = False
        return
    q = p["qs"][p["at"]]
    n = len(q["options"])
    if k.isdigit() and 1 <= int(k) <= n:
        i = int(k) - 1
        if q["multi"]:
            c = p["checks"][p["at"]]
            c.remove(i) if i in c else c.append(i)
        else:
            p["single"][str(p["at"])] = i
            _next(p)
    elif k == "down":
        p["focus"] = min(p["focus"] + 1, n + 1 if q["multi"] else n)
    elif k == "tab":
        _next(p)
    elif k == "left" and p["at"] > 0:
        p["at"], p["focus"] = p["at"] - 1, 0
    elif k == "right":
        if p["at"] + 1 < len(p["qs"]):
            p["at"], p["focus"] = p["at"] + 1, 0
        else:
            p["review"] = True


def handle(conn):
    buf = b""
    try:
        while b"\n" not in buf:
            chunk = conn.recv(65536)
            if not chunk:
                return
            buf += chunk
        req = json.loads(buf.split(b"\n", 1)[0])
        method, params, rid = req.get("method"), req.get("params") or {}, req.get("id")
        delay = 0
        with lock:
            with open(calls_path, "a") as f:
                f.write(json.dumps({"method": method, "params": params}, sort_keys=True) + "\n")
            st = load()
            addr = params.get("terminal", "")
            if method == "terminal.list":
                reply = {"result": {"terminals": st["terminals"], "hidden_finished": 0, "machines": []}}
            elif method == "terminal.read":
                if addr in st.get("pickers", {}):
                    reply = {"result": {"text": render(st["pickers"][addr]), "complete": True, "how": "screen", "why": None}}
                elif addr in st.get("screens", {}):
                    reply = {"result": {"text": st["screens"][addr], "complete": True, "how": "screen", "why": None}}
                else:
                    reply = {"error": {"code": -32000, "message": "no terminal " + addr, "data": {"kind": "not_found"}}}
            elif method in ("terminal.keys", "terminal.send"):
                if method == "terminal.keys" and addr in st.get("pickers", {}):
                    for k in params.get("keys") or []:
                        press(st["pickers"][addr], k)
                    save(st)
                elif method == "terminal.keys" and addr in st.get("after_keys", {}):
                    st["screens"][addr] = st["after_keys"].pop(addr)
                    save(st)
                reply = {"result": {"turn": 0, "status": "idle", "started": True}}
                if method == "terminal.keys":
                    delay = st.get("delay_keys", {}).get(addr, 0)
            elif method == "terminal.put":
                reply = {"result": {"path": "/remote/tmp/" + str(params.get("name", ""))}}
            elif method == "doc.get":
                reply = {"result": {"value": None}}
            elif method == "machine.list":
                reply = {"result": {"machines": []}}
            else:
                reply = {"error": {"code": -32601, "message": "unknown method " + str(method)}}
        if delay:
            time.sleep(delay)  # outside the lock: other calls go on
        reply.update({"jsonrpc": "2.0", "id": rid})
        conn.sendall((json.dumps(reply) + "\n").encode())
    finally:
        conn.close()


if os.path.exists(sock_path):
    os.unlink(sock_path)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
srv.listen(16)
print("ready", flush=True)
while True:
    c, _ = srv.accept()
    threading.Thread(target=handle, args=(c,), daemon=True).start()
