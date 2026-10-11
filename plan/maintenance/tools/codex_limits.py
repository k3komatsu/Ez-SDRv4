"""Print the Codex account rate limits (5-hour and weekly windows) without running a model turn.

Uses `codex app-server`'s `account/rateLimits/read` (codex-cli 0.160). AGENTS.md §9: Sol draws on this quota."""
import json, subprocess, sys, time, datetime
p = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
def send(m): p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
send({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "limits-probe", "version": "0"}}})
send({"method": "initialized"})
send({"id": 2, "method": "account/rateLimits/read", "params": {"excludeResetCreditDetails": True}})
deadline = time.time() + 30
while time.time() < deadline:
    line = p.stdout.readline()
    if not line: break
    m = json.loads(line)
    if m.get("id") == 2:
        r = m.get("result") or m
        def show(name, w):
            if not w: return
            ts = w.get("resetsAt") or w.get("resets_at")
            when = datetime.datetime.fromtimestamp(ts).strftime("%m-%d %H:%M") if ts else "?"
            print(f"{name}: used {w.get('usedPercent', w.get('used_percent'))}% / window {w.get('windowDurationMins', w.get('window_minutes'))} min / resets {when}")
        rl = r.get("rateLimits") or {}
        show("primary (5h)", rl.get("primary")); show("secondary (week)", rl.get("secondary"))
        if not rl: print(json.dumps(r)[:1500])
        break
p.kill()
