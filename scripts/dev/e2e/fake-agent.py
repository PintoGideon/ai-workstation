#!/usr/bin/env python3
"""Fake agent CLI for AIW end-to-end runs.

Installed under five names (`claude`, `codex`, `gemini`, `cursor-agent`, `pi`)
on a PATH prefix. It behaves like an interactive REPL and fires the hooks AIW
installs into the checkout exactly the way the real CLIs do:

- claude       reads `.claude/settings.local.json`  (hooks.<Event>[].hooks[].command + args)
- codex        takes `-c notify=[...]` and runs that argv with the JSON payload appended,
               and reads `.codex/hooks.json`       (hooks.<Event>[].hooks[].command)
- gemini       reads `.gemini/settings.json`        (hooks.<Event>[].hooks[].command)
- cursor-agent reads `.cursor/hooks.json`           (hooks.<event>[].command)
- pi           has no hooks

Commands typed into the REPL (deterministic, scriptable):

  /ask     emit a permission request (Notification), print a yes/no prompt
  /perm C  run the PermissionRequest hook for `Bash C` and print
           `permission: allow|deny|prompt` from its answer
  /done    emit the turn-end hook (Stop / AfterAgent / stop / codex notify)
  /work    emit prompt-submit + tool-start and stay "working"
  /tool    emit tool-start then tool-stop
  /crash   exit 1
  /spam    print 2000 lines as fast as possible
  /wide    print long unicode / emoji / RTL / control-sequence lines
  /slow    sleep 20 s ignoring input
  /sleep N sleep N seconds
  /exit    emit session-end and exit 0
  <text>   a normal turn: prompt-submit, thinking, tool start/stop, answer, stop

Every launch appends its argv to `$AIW_E2E_LOG/<name>.args` and every hook run
to `$AIW_E2E_LOG/<name>.hooks` so a driver can assert on them.
"""

import json
import os
import re
import shlex
import subprocess
import sys
import time
import uuid

NAME = os.path.basename(sys.argv[0])
ARGS = sys.argv[1:]
CWD = os.getcwd()
HOME = os.environ.get("HOME", "/tmp")
LOG_DIR = os.environ.get("AIW_E2E_LOG") or os.path.join(HOME, ".e2e-log")
HOOK_TIMEOUT = float(os.environ.get("AIW_E2E_HOOK_TIMEOUT", "15"))


def log(file, record):
    try:
        os.makedirs(LOG_DIR, exist_ok=True)
        with open(os.path.join(LOG_DIR, f"{NAME}.{file}"), "a", encoding="utf-8") as fh:
            fh.write(json.dumps(record, ensure_ascii=False) + "\n")
    except OSError:
        pass


def out(text=""):
    sys.stdout.write(text + "\n")
    sys.stdout.flush()


# ---------------------------------------------------------------- arguments
if "--version" in ARGS or "-V" in ARGS or ARGS[:1] == ["--version"]:
    out(f"{NAME} 1.0.0 (fake e2e agent)")
    sys.exit(0)
if "--help" in ARGS or "-h" in ARGS:
    out(f"usage: {NAME} [options]\n  --version\n  --resume <id>\n(fake e2e agent)")
    sys.exit(0)

log("args", {"ts": time.time(), "cwd": CWD, "args": ARGS, "env": {
    k: v for k, v in os.environ.items() if k.startswith("AIW_")}})

resume_id = None
notify_argv = []
it = iter(range(len(ARGS)))
i = 0
while i < len(ARGS):
    a = ARGS[i]
    if a in ("--resume", "-r") and i + 1 < len(ARGS):
        resume_id = ARGS[i + 1]
        i += 2
        continue
    if NAME == "codex" and a == "resume" and i + 1 < len(ARGS):
        resume_id = ARGS[i + 1]
        i += 2
        continue
    if a == "-c" and i + 1 < len(ARGS):
        kv = ARGS[i + 1]
        if kv.startswith("notify="):
            notify_argv = re.findall(r'"((?:[^"\\]|\\.)*)"', kv[len("notify="):])
            notify_argv = [s.encode().decode("unicode_escape") for s in notify_argv]
        i += 2
        continue
    i += 1

SESSION_ID = resume_id or str(uuid.uuid4())


# ---------------------------------------------------------------- transcripts
def encode_path(path):
    return "".join(c if c.isalnum() and c.isascii() else "-" for c in path)


def claude_transcript_path():
    d = os.path.join(HOME, ".claude", "projects", encode_path(CWD))
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, f"{SESSION_ID}.jsonl")


def codex_rollout_path():
    now = time.gmtime()
    d = os.path.join(HOME, ".codex", "sessions", time.strftime("%Y/%m/%d", now))
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, f"rollout-{time.strftime('%Y-%m-%dT%H-%M-%S', now)}-{SESSION_ID}.jsonl")
    if not os.path.exists(path):
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(json.dumps({
                "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", now),
                "type": "session_meta",
                "payload": {"id": SESSION_ID, "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", now),
                            "cwd": CWD, "originator": "codex_cli_rs", "cli_version": "1.0.0",
                            "instructions": None, "source": "cli"},
            }) + "\n")
    return path


def transcript_path():
    if NAME == "claude":
        return claude_transcript_path()
    if NAME == "codex":
        return codex_rollout_path()
    d = os.path.join(HOME, f".{NAME}", "transcripts")
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, f"{SESSION_ID}.jsonl")


def record_turn(prompt, answer):
    path = transcript_path()
    ts = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
    with open(path, "a", encoding="utf-8") as fh:
        if NAME == "codex":
            fh.write(json.dumps({"timestamp": ts, "type": "response_item", "payload": {
                "type": "message", "role": "user", "content": [{"type": "input_text", "text": prompt}]}}) + "\n")
            fh.write(json.dumps({"timestamp": ts, "type": "response_item", "payload": {
                "type": "message", "role": "assistant", "content": [{"type": "output_text", "text": answer}]}}) + "\n")
        else:
            fh.write(json.dumps({"type": "user", "sessionId": SESSION_ID, "cwd": CWD, "timestamp": ts,
                                 "message": {"role": "user", "content": prompt}}) + "\n")
            fh.write(json.dumps({"type": "assistant", "sessionId": SESSION_ID, "cwd": CWD, "timestamp": ts,
                                 "message": {"role": "assistant", "model": "fake-model-1",
                                             "content": [{"type": "text", "text": answer}],
                                             "usage": {"input_tokens": 120, "output_tokens": 40}}}) + "\n")


# ---------------------------------------------------------------- hooks
def read_json(path):
    try:
        with open(path, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return None


def hook_commands(event):
    """[(command string, run-with-shell)] configured for `event` in the checkout."""
    if NAME == "claude":
        cfg = read_json(os.path.join(CWD, ".claude", "settings.local.json")) or {}
        cmds = []
        for group in (cfg.get("hooks") or {}).get(event, []) or []:
            for h in group.get("hooks", []) or []:
                if h.get("type", "command") != "command":
                    continue
                cmd = h.get("command", "")
                args = h.get("args") or []
                if args:
                    cmd = cmd + " " + " ".join(shlex.quote(str(a)) for a in args)
                cmds.append(cmd)
        return cmds
    if NAME == "codex":
        cfg = read_json(os.path.join(CWD, ".codex", "hooks.json")) or {}
        cmds = []
        for group in (cfg.get("hooks") or {}).get(event, []) or []:
            for h in group.get("hooks", []) or []:
                if h.get("type", "command") == "command" and h.get("command"):
                    cmds.append(h["command"])
        return cmds
    if NAME == "gemini":
        cfg = read_json(os.path.join(CWD, ".gemini", "settings.json")) or {}
        cmds = []
        for group in (cfg.get("hooks") or {}).get(event, []) or []:
            for h in group.get("hooks", []) or []:
                if h.get("type", "command") == "command" and h.get("command"):
                    cmds.append(h["command"])
        return cmds
    if NAME == "cursor-agent":
        cfg = read_json(os.path.join(CWD, ".cursor", "hooks.json")) or {}
        return [h["command"] for h in (cfg.get("hooks") or {}).get(event, []) or [] if h.get("command")]
    return []


def base_payload(event):
    if NAME == "claude":
        return {"session_id": SESSION_ID, "transcript_path": transcript_path(), "cwd": CWD,
                "hook_event_name": event, "permission_mode": "default"}
    if NAME == "codex":
        return {"session_id": SESSION_ID, "transcript_path": transcript_path(), "cwd": CWD,
                "hook_event_name": event, "model": "fake-model-1", "turn_id": str(uuid.uuid4())}
    if NAME == "gemini":
        return {"session_id": SESSION_ID, "transcript_path": transcript_path(), "cwd": CWD,
                "hook_event_name": event, "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
    if NAME == "cursor-agent":
        p = {"conversation_id": SESSION_ID, "generation_id": str(uuid.uuid4()), "hook_event_name": event,
             "workspace_roots": [CWD], "transcript_path": transcript_path()}
        if event == "sessionStart":
            p["session_id"] = SESSION_ID
        return p
    return {}


def run_hook(cmd, payload_text, event):
    try:
        proc = subprocess.run(["sh", "-c", cmd], input=payload_text.encode("utf-8"),
                              capture_output=True, timeout=HOOK_TIMEOUT, cwd=CWD)
        rec = {"ts": time.time(), "event": event, "cmd": cmd, "code": proc.returncode,
               "stdout": proc.stdout.decode("utf-8", "replace")[:2000],
               "stderr": proc.stderr.decode("utf-8", "replace")[:400]}
    except subprocess.TimeoutExpired:
        rec = {"ts": time.time(), "event": event, "cmd": cmd, "code": None, "timeout": True}
    log("hooks", rec)
    return rec


def fire_collect(event, extra=None):
    """Fire `event` through the hook files (never the Codex notify argv) and
    return the stdout of every hook, in order."""
    cmds = hook_commands(event)
    if not cmds:
        log("hooks", {"ts": time.time(), "event": event, "cmd": None, "note": "no hook configured"})
        return []
    payload = base_payload(event)
    payload.update(extra or {})
    text = json.dumps(payload)
    return [run_hook(cmd, text, event).get("stdout", "") for cmd in cmds]


def first_json(outputs):
    for text in outputs:
        text = (text or "").strip()
        if not text:
            continue
        try:
            return json.loads(text)
        except ValueError:
            continue
    return None


def permission_request(command):
    """Run the harness's permission hook for `Bash <command>` and say what it decided."""
    if NAME == "claude" or NAME == "codex":
        answer = first_json(fire_collect("PermissionRequest", {
            "tool_name": "Bash", "tool_input": {"command": command, "description": "e2e"}}))
        behavior = ((answer or {}).get("hookSpecificOutput") or {}).get("decision", {}).get("behavior")
        return {"allow": "allow", "deny": "deny"}.get(behavior, "prompt")
    if NAME == "cursor-agent":
        answer = first_json(fire_collect("beforeShellExecution", {"command": command, "cwd": CWD}))
        permission = (answer or {}).get("permission")
        return {"allow": "allow", "deny": "deny"}.get(permission, "prompt")
    if NAME == "gemini":
        ev("ask", ask_extra())
    return "prompt"


def fire(event, extra=None):
    """Fire `event` with the agent's native payload shape."""
    extra = extra or {}
    if NAME == "codex":
        if event != "turn-complete" or not notify_argv:
            return
        payload = {"type": "agent-turn-complete", "thread-id": SESSION_ID, "turn-id": str(uuid.uuid4()),
                   "input-messages": [extra.get("prompt", "")], "last-assistant-message": extra.get("answer", "")}
        argv = notify_argv + [json.dumps(payload)]
        try:
            proc = subprocess.run(argv, capture_output=True, timeout=HOOK_TIMEOUT, cwd=CWD)
            log("hooks", {"ts": time.time(), "event": event, "cmd": argv[0], "code": proc.returncode,
                          "stderr": proc.stderr.decode("utf-8", "replace")[:400]})
        except (subprocess.TimeoutExpired, OSError) as err:
            log("hooks", {"ts": time.time(), "event": event, "cmd": argv[0], "code": None, "error": str(err)})
        return
    cmds = hook_commands(event)
    if not cmds:
        log("hooks", {"ts": time.time(), "event": event, "cmd": None, "note": "no hook configured"})
        return
    payload = base_payload(event)
    payload.update(extra)
    text = json.dumps(payload)
    for cmd in cmds:
        run_hook(cmd, text, event)


EVENTS = {
    "claude": {"start": "SessionStart", "prompt": "UserPromptSubmit", "tool_start": "PreToolUse",
               "tool_stop": "PostToolUse", "ask": "Notification", "stop": "Stop", "end": "SessionEnd"},
    "gemini": {"start": "SessionStart", "prompt": "BeforeAgent", "tool_start": "BeforeTool",
               "tool_stop": "AfterTool", "ask": "Notification", "stop": "AfterAgent", "end": "SessionEnd"},
    "cursor-agent": {"start": "sessionStart", "prompt": "beforeSubmitPrompt", "tool_start": "preToolUse",
                     "tool_stop": "postToolUse", "ask": None, "stop": "stop", "end": "sessionEnd"},
    "codex": {"start": None, "prompt": None, "tool_start": None, "tool_stop": None, "ask": None,
              "stop": "turn-complete", "end": None},
    "pi": {},
}
EV = EVENTS.get(NAME, {})


def ev(kind, extra=None):
    name = EV.get(kind)
    if name:
        fire(name, extra)


def tool_extra(kind):
    if NAME == "claude":
        d = {"tool_name": "Bash", "tool_input": {"command": "cargo test", "description": "run tests"},
             "tool_use_id": "toolu_" + uuid.uuid4().hex[:12]}
        if kind == "stop":
            d["tool_response"] = {"stdout": "ok", "stderr": "", "interrupted": False}
        return d
    if NAME == "gemini":
        d = {"tool_name": "run_shell_command", "tool_input": {"command": "cargo test"}}
        if kind == "stop":
            d["tool_response"] = {"output": "ok"}
        return d
    if NAME == "cursor-agent":
        d = {"tool_name": "Shell", "tool_input": {"command": "cargo test"}}
        if kind == "stop":
            d["tool_output"] = {"stdout": "ok"}
        return d
    return {}


def ask_extra():
    if NAME == "claude":
        return {"notification_type": "permission_prompt",
                "message": "Claude needs your permission to use Bash", "title": "Permission needed"}
    if NAME == "gemini":
        return {"notification_type": "ToolPermission", "message": "Gemini needs approval to run cargo test"}
    return {}


# ---------------------------------------------------------------- REPL
def banner():
    out(f"╭─ {NAME} (fake e2e agent) ─────────────────────────────────╮")
    out(f"│ session {SESSION_ID}")
    out(f"│ cwd     {CWD}")
    out(f"│ args    {' '.join(shlex.quote(a) for a in ARGS) or '(none)'}")
    if resume_id:
        out(f"│ resumed conversation {resume_id}")
    out("│ commands: /ask /perm /done /work /tool /crash /spam /wide /slow /exit")
    out("╰──────────────────────────────────────────────────────────────╯")


def normal_turn(prompt):
    ev("prompt", {"prompt": prompt})
    out("⠋ thinking…")
    time.sleep(0.15)
    ev("tool_start", tool_extra("start"))
    out("⏺ Bash(cargo test)")
    out("  ⎿  running 3 tests … ok")
    time.sleep(0.15)
    ev("tool_stop", tool_extra("stop"))
    answer = f"Done: handled '{prompt}' ({NAME})."
    out(f"⏺ {answer}")
    record_turn(prompt, answer)
    ev("stop", {"prompt": prompt, "answer": answer, "stop_hook_active": False, "status": "completed"})


def wide_lines():
    lines = [
        "🚀 emoji ✅ 🧪 🔥 🐍 🦀 " * 12,
        "漢字かなカナ한국어日本語テキスト " * 10,
        "RTL: مرحبا بالعالم שלום עולם " * 8,
        "combining: é ä ñ z͡a͜l͡g͡o " * 8,
        "zero-width:​‌‍﻿ joined ‍👨‍👩‍👧‍👦 👩🏽‍💻 🏳️‍🌈 " * 6,
        "\x1b]0;fake title ✨\x07title-set \x1b[1;31mred\x1b[0m \x1b[38;2;10;200;10mtruecolor\x1b[0m \x1b[7mreverse\x1b[0m",
        "\x1b[3Acursor-up garbage \x1b[K\x1b[?25l hidden cursor \x1b[?25h \a bell \x1b[2J\x1b[H cleared",
        "tabs\there\tand\there\t\tend " + "─" * 150,
        "very-long-" + "x" * 400,
    ]
    for _ in range(3):
        for line in lines:
            out(line)


def main():
    banner()
    ev("start", {"source": "startup"})
    while True:
        try:
            sys.stdout.write("❯ ")
            sys.stdout.flush()
            line = sys.stdin.readline()
        except KeyboardInterrupt:
            out("^C (type /exit to quit)")
            continue
        if line == "":
            out("(eof) bye")
            ev("end", {"reason": "exit"})
            return 0
        line = line.strip()
        if not line:
            continue
        cmd, _, rest = line.partition(" ")
        if cmd == "/perm":
            command = rest or "cargo test"
            out(f"⏺ Bash({command})")
            out(f"permission: {permission_request(command)}")
        elif cmd == "/ask":
            out("⏺ Bash(rm -rf build/)")
            out("Do you want to proceed?")
            out("❯ 1. Yes   2. Yes, and don't ask again   3. No")
            ev("ask", ask_extra())
        elif cmd == "/done":
            answer = "Finished the task."
            out(f"⏺ {answer}")
            ev("stop", {"prompt": rest or "(previous)", "answer": answer, "stop_hook_active": False,
                        "status": "completed"})
        elif cmd == "/work":
            ev("prompt", {"prompt": rest or "work"})
            out("⠋ thinking…")
            ev("tool_start", tool_extra("start"))
            out("⏺ Bash(cargo build) … still running")
        elif cmd == "/tool":
            ev("tool_start", tool_extra("start"))
            out("⏺ Bash(cargo test)")
            ev("tool_stop", tool_extra("stop"))
            out("  ⎿  ok")
        elif cmd == "/crash":
            sys.stderr.write("fatal: simulated crash\n")
            sys.stderr.flush()
            sys.exit(1)
        elif cmd == "/spam":
            for n in range(2000):
                out(f"{n:05d} lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor")
            out("spam done")
        elif cmd == "/wide":
            wide_lines()
            out("wide done")
        elif cmd == "/slow":
            out("sleeping 20 s …")
            time.sleep(20)
            out("awake")
        elif cmd == "/sleep":
            try:
                time.sleep(float(rest or "1"))
            except ValueError:
                pass
            out("awake")
        elif cmd in ("/exit", "/quit", "exit", "quit"):
            ev("end", {"reason": "exit"})
            out("bye")
            return 0
        else:
            normal_turn(line)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except BrokenPipeError:
        sys.exit(0)
