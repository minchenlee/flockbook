"""A fake herdr-core on a Unix socket for the modfeed parity check (no real hc is ever touched).

Plain-mode JSON-RPC, one request per connection, as the sidecars use it. State comes from a JSON file that the
driver rewrites between commands: {"terminals": [...], "screens": {addr: text}, "after_keys": {addr: text}}.
Every call is appended to calls.jsonl as {"method", "params"}. `terminal.keys` moves the screen to `after_keys[addr]`
when one is set (a prompt that goes away once answered).
"""
import json, os, socket, sys, threading

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
        with lock:
            with open(calls_path, "a") as f:
                f.write(json.dumps({"method": method, "params": params}, sort_keys=True) + "\n")
            st = load()
            addr = params.get("terminal", "")
            if method == "terminal.list":
                reply = {"result": {"terminals": st["terminals"], "hidden_finished": 0, "machines": []}}
            elif method == "terminal.read":
                if addr in st.get("screens", {}):
                    reply = {"result": {"text": st["screens"][addr], "complete": True, "how": "screen", "why": None}}
                else:
                    reply = {"error": {"code": -32000, "message": "no terminal " + addr, "data": {"kind": "not_found"}}}
            elif method in ("terminal.keys", "terminal.send"):
                if method == "terminal.keys" and addr in st.get("after_keys", {}):
                    st["screens"][addr] = st["after_keys"].pop(addr)
                    save(st)
                reply = {"result": {"turn": 0, "status": "idle", "started": True}}
            elif method == "doc.get":
                reply = {"result": {"value": None}}
            elif method == "machine.list":
                reply = {"result": {"machines": []}}
            else:
                reply = {"error": {"code": -32601, "message": "unknown method " + str(method)}}
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
