#!/usr/bin/env python3
"""Shared model client for the background daemons. Every caller reads the active
profile in fsagent/model.json, so `fsmodel <profile>` swaps the whole stack."""

import json
import os
import sys
import time
import urllib.request

MODEL_JSON = os.path.expanduser("~/local-agent/fsagent/model.json")
USAGE_LOG = os.path.expanduser("~/local-agent/logs/llm-usage.log")
# per-MTok input/output, for the --usage rollup only
RATES = {"claude-haiku-4-5-20251001": (1.0, 5.0), "claude-sonnet-5": (3.0, 15.0)}
DEFAULT = {"profile": "local", "url": "http://127.0.0.1:8015/v1/chat/completions",
           "model": "gpt-oss-20b", "key": ""}
_CACHE = {"mtime": -1, "cfg": dict(DEFAULT)}


def config():
    """Active backend, re-read whenever model.json changes."""
    try:
        mtime = os.stat(MODEL_JSON).st_mtime
    except OSError:
        mtime = None
    if _CACHE["mtime"] != mtime:
        cfg = dict(DEFAULT)
        if mtime is not None:
            try:
                d = json.load(open(MODEL_JSON))
                p = d["profiles"][d["active"]]
                cfg = {"profile": d["active"], "url": p["url"],
                       "model": p["model"], "key": (p.get("key") or "").strip()}
                kf = p.get("key_file")
                if kf and not cfg["key"]:
                    try:
                        cfg["key"] = open(os.path.expanduser(kf)).read().strip()
                    except OSError:
                        pass  # key not dropped yet; the endpoint 401s with a clear error
            except (ValueError, KeyError, TypeError, OSError):
                pass  # malformed config falls back to the local lane
        _CACHE["mtime"], _CACHE["cfg"] = mtime, cfg
    cfg = dict(_CACHE["cfg"])
    cfg["local"] = "127.0.0.1" in cfg["url"] or "localhost" in cfg["url"]
    return cfg


def is_local():
    return config()["local"]


def label():
    cfg = config()
    return "%s/%s" % (cfg["profile"], cfg["model"])


def complete(messages, max_tokens=600, temperature=0.2, timeout=120, effort="low"):
    """One chat completion, returning the assistant text. Raises on transport error."""
    cfg = config()
    body = {"model": cfg["model"], "messages": messages, "max_tokens": max_tokens}
    if cfg["local"]:
        # llama-server knob; cloud endpoints reject it
        body["temperature"] = temperature
        body["chat_template_kwargs"] = {"reasoning_effort": effort}
    elif cfg["model"].startswith("claude-opus-5"):
        # opus 5 rejects temperature outright; no-think keeps the daemons cheap
        body["reasoning_effort"] = "none"
    else:
        body["temperature"] = temperature
    headers = {"Content-Type": "application/json"}
    if cfg["key"]:
        headers["Authorization"] = "Bearer " + cfg["key"]
    req = urllib.request.Request(cfg["url"], json.dumps(body).encode(), headers)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        out = json.load(r)
    _log_usage(cfg, out.get("usage") or {})
    return (out["choices"][0]["message"].get("content") or "").strip()


def _log_usage(cfg, usage):
    """One tab-separated line per call; --usage rolls the day up. Never fatal."""
    try:
        with open(USAGE_LOG, "a") as f:
            f.write("%s\t%s\t%s\t%s\t%d\t%d\n" % (
                time.strftime("%Y-%m-%dT%H:%M:%S"), os.path.basename(sys.argv[0]),
                cfg["profile"], cfg["model"],
                usage.get("prompt_tokens", 0), usage.get("completion_tokens", 0)))
    except OSError:
        pass


def usage_report(day=None):
    day = day or time.strftime("%Y-%m-%d")
    rows, total = {}, [0, 0, 0.0]
    try:
        lines = open(USAGE_LOG).read().splitlines()
    except OSError:
        return "no usage log yet"
    for ln in lines:
        p = ln.split("\t")
        if len(p) != 6 or not p[0].startswith(day):
            continue
        rin, rout = RATES.get(p[3], (0.0, 0.0))
        cost = int(p[4]) / 1e6 * rin + int(p[5]) / 1e6 * rout
        r = rows.setdefault(p[1], [0, 0, 0, 0.0])
        r[0] += 1
        r[1] += int(p[4])
        r[2] += int(p[5])
        r[3] += cost
        total[0] += int(p[4])
        total[1] += int(p[5])
        total[2] += cost
    if not rows:
        return "no calls logged on " + day
    out = ["%s  calls   in-tok   out-tok    cost" % day]
    for name, r in sorted(rows.items(), key=lambda kv: -kv[1][3]):
        out.append("%-24s %5d %8d %8d   $%.3f" % (name, r[0], r[1], r[2], r[3]))
    out.append("%-24s %5s %8d %8d   $%.3f"
               % ("TOTAL", "", total[0], total[1], total[2]))
    return "\n".join(out)


def json_complete(messages, max_tokens=1200, temperature=0.1, timeout=120):
    """complete() plus the JSON-object extraction every caller was repeating."""
    text = complete(messages, max_tokens=max_tokens, temperature=temperature,
                    timeout=timeout)
    dec, start = json.JSONDecoder(), text.find("{")
    while start != -1:
        try:
            cand, _ = dec.raw_decode(text[start:])
            if isinstance(cand, dict):
                return cand
        except ValueError:
            pass
        start = text.find("{", start + 1)
    raise ValueError("no JSON object in model output: " + text[:200])


if __name__ == "__main__":
    if "--usage" in sys.argv:
        arg = [a for a in sys.argv[1:] if not a.startswith("-")]
        print(usage_report(arg[0] if arg else None))
    else:
        cfg = config()
        print("profile=%s model=%s local=%s key=%s"
              % (cfg["profile"], cfg["model"], cfg["local"], bool(cfg["key"])))
        print(complete([{"role": "user", "content": "Reply with the word ready."}],
                       max_tokens=16, timeout=30))
