"""Per-session task timelines for herdr's detail panel (Josh 2026-08-27).

Each live Claude session gets ~/.local/state/herdr-detail/<sid>.json holding
done / in-flight / inferred-next task lines with anchors, per-task seconds
and session aggregates. The panel renders it; clicking an anchor jumps the
conversation view. Tasks map to real user prompts: everything before the
last prompt is done; the last one is in flight while the pane reports
working. Purple "next" lines are the model's guess at what remains.
"""

import datetime
import json
import logging
import os
import time

log = logging.getLogger("summarizer.timeline")

OUT_DIR = os.path.join(os.path.expanduser("~"), ".local", "state", "herdr-detail")
HEAD_CHARS = 240
LABEL_WORDS_MAX = 6
NEXT_COOLDOWN_SECS = 180
MAX_LLM_CALLS_PER_PASS = 6
PRUNE_AGE_SECS = 7 * 86400

NOISE_PREFIXES = ("<", "Caveat:", "[SYSTEM", "[Request interrupted")

_llm = None
_find_transcript = None


def configure(llm_module, find_transcript):
    global _llm, _find_transcript
    _llm = llm_module
    _find_transcript = find_transcript


def _path(sid):
    return os.path.join(OUT_DIR, sid + ".json")


def _load(sid):
    try:
        with open(_path(sid)) as f:
            return json.load(f)
    except Exception:
        return {}


def _save(sid, data):
    os.makedirs(OUT_DIR, exist_ok=True)
    tmp = _path(sid) + ".tmp"
    with open(tmp, "w") as f:
        json.dump(data, f)
    os.replace(tmp, _path(sid))


def _epoch(iso):
    try:
        return datetime.datetime.fromisoformat(iso.replace("Z", "+00:00")).timestamp()
    except Exception:
        return 0.0


def _text_of(message):
    content = (message or {}).get("content")
    if isinstance(content, str):
        return content.strip()
    if isinstance(content, list):
        parts = [b.get("text", "") for b in content
                 if isinstance(b, dict) and b.get("type") == "text"]
        return "\n".join(p for p in parts if p).strip()
    return ""


def _is_prompt(rec):
    if rec.get("type") != "user" or rec.get("isMeta"):
        return False
    text = _text_of(rec.get("message"))
    if not text or text.startswith(NOISE_PREFIXES):
        return False
    if text.startswith("/") and " " not in text.split("\n", 1)[0]:
        return False
    if text.startswith("This session is being continued from"):
        return False
    return True


def _parse_new(tpath, data):
    """Append exchanges from bytes past the stored offset; restart on shrink."""
    size = os.path.getsize(tpath)
    p = data.get("_p") or {}
    off = p.get("off", 0)
    if size < p.get("size", 0) or off > size:
        off, data["_exch"] = 0, []
    exch = data.setdefault("_exch", [])
    with open(tpath, "rb") as f:
        f.seek(off)
        blob = f.read()
    consumed = off
    for raw in blob.split(b"\n"):
        line_off = consumed
        consumed += len(raw) + 1
        if consumed > size + 1:
            break
        try:
            rec = json.loads(raw)
        except Exception:
            continue
        ts = _epoch(rec.get("timestamp", ""))
        if _is_prompt(rec):
            text = _text_of(rec.get("message"))
            exch.append({"u": rec.get("uuid") or f"off{line_off}", "ts": ts,
                         "off": line_off, "head": text[:HEAD_CHARS],
                         "rhead": "", "out": 0, "last_ts": ts})
        elif exch and rec.get("type") == "assistant":
            cur = exch[-1]
            usage = (rec.get("message") or {}).get("usage") or {}
            cur["out"] += usage.get("output_tokens") or 0
            reply = _text_of(rec.get("message"))
            if reply:
                cur["rhead"] = reply[:HEAD_CHARS]
            if ts:
                cur["last_ts"] = max(cur["last_ts"], ts)
        elif exch and ts:
            exch[-1]["last_ts"] = max(exch[-1]["last_ts"], ts)
    data["_p"] = {"off": min(consumed, size), "size": size}
    return size


def _label_call(system, user):
    line = _llm.complete(
        [{"role": "system", "content": system},
         {"role": "user", "content": user}],
        max_tokens=300, timeout=45)
    words = line.strip().strip('"').replace("\n", " ").split()
    return " ".join(words[:LABEL_WORDS_MAX])


BATCH_LABEL_MAX = 10


def _label_done_batch(exchanges):
    """One call labels up to BATCH_LABEL_MAX finished exchanges; a long
    session backfills in one pass instead of dribbling out over minutes."""
    numbered = "\n".join(
        f"[{i}] prompt: {ex['head'][:200]}\n    reply: {(ex.get('rhead') or '(none)')[:200]}"
        for i, ex in enumerate(exchanges))
    raw = _llm.complete(
        [{"role": "system", "content":
          "You compress finished coding-agent exchanges into labels. The "
          "user message numbers each exchange. Reply with a JSON array of "
          "strings, one per exchange in the same order, each a two-to-six "
          "word past-tense clause naming what got done. Plain words. "
          "Nothing but the JSON array."},
         {"role": "user", "content": numbered}],
        max_tokens=1500, timeout=90)
    start, end = raw.find("["), raw.rfind("]")
    if start < 0 or end <= start:
        return None
    labels = json.loads(raw[start:end + 1])
    return [" ".join(str(s).split()[:LABEL_WORDS_MAX]) for s in labels]


def _label_current(ex):
    return _label_call(
        "You compress a coding task that is currently in progress into a "
        "two-to-six word present-tense clause. Plain words, no punctuation, "
        "no quotes.",
        f"prompt: {ex['head']}")


def _infer_next(done_labels, current_label, latest_reply):
    raw = _llm.complete(
        [{"role": "system", "content":
          "You watch a coding project's task history and predict what "
          "remains. Reply with a JSON array of one to four strings, each a "
          "two-to-six word future step likely still needed before the "
          "overall project is complete. Nothing but the JSON array."},
         {"role": "user", "content":
          "completed:\n" + "\n".join(done_labels[-12:] or ["(none)"]) +
          "\nin flight:\n" + (current_label or "(none)") +
          "\nlatest reply tail:\n" + (latest_reply[-600:] or "(none)")}],
        max_tokens=400, timeout=60)
    start, end = raw.find("["), raw.rfind("]")
    if start < 0 or end <= start:
        return None
    items = json.loads(raw[start:end + 1])
    return [" ".join(str(s).split()[:LABEL_WORDS_MAX])
            for s in items if str(s).strip()][:4]


def _publish(data, status):
    """Task and session times are ACTIVE time (first to last activity inside
    each exchange), so a prompt that sat overnight is not an 814-minute task."""
    exch = data.get("_exch") or []
    working = status == "working"
    done_n = len(exch) - 1 if (exch and working) else len(exch)
    done, current = [], []
    for i, ex in enumerate(exch):
        active = max(0, ex["last_ts"] - ex["ts"])
        secs = None if i >= done_n else active
        # no fallback to raw prompt words: an empty label renders as a
        # placeholder until the model names it (Josh 2026-08-27); head rides
        # along so a click can find the exchange in the terminal feed
        item = {"u": ex["u"], "ts": ex["ts"], "off": ex["off"], "secs": secs,
                "head": ex["head"][:80],
                "label": ex.get("label") or ex.get("ylabel") or ""}
        (done if i < done_n else current).append(item)
    data["done"], data["current"] = done, current
    data["status"] = status
    data["total_secs"] = sum(max(0, ex["last_ts"] - ex["ts"]) for ex in exch)
    data["out_tokens"] = sum(ex["out"] for ex in exch)
    data["updated"] = time.time()
    data["v"] = 1


def _fmt_event(evt):
    """["2026-08-27T18:59:47Z", "tool", "Write  /path/to/file.ts", id] ->
    "18:59 Write file.ts", local time."""
    try:
        ts = datetime.datetime.fromisoformat(str(evt[0]).replace("Z", "+00:00"))
        clock = ts.astimezone().strftime("%H:%M")
    except Exception:
        clock = "--:--"
    detail = str(evt[2]) if len(evt) > 2 else ""
    parts = detail.split()
    tool = parts[0] if parts else str(evt[1]) if len(evt) > 1 else ""
    target = os.path.basename(parts[-1]) if len(parts) > 1 else ""
    return " ".join(x for x in (clock, tool, target[:40]) if x)


def _subs_for_tab(lanes, tab_id):
    rows = sorted(lanes.get(tab_id) or [], key=lambda r: r.get("started") or 0)
    return [{"label": r["label"], "started": r.get("started") or 0,
             "path": r.get("path") or "",
             "events": [_fmt_event(e) for e in (r.get("events") or [])[-5:]][::-1]}
            for r in rows]


def process(snapshot, tries, allow_llm, lanes=None):
    if _llm is None or not snapshot:
        return
    budget = MAX_LLM_CALLS_PER_PASS if allow_llm else 0
    live = set()
    for pane in snapshot.get("panes", []):
        sess = pane.get("agent_session") or {}
        sid = sess.get("value") or ""
        if pane.get("agent") != "claude" or sess.get("kind") != "id" or not sid:
            continue
        live.add(sid)
        tpath = _find_transcript(sid)
        if not tpath:
            continue
        subs = _subs_for_tab(lanes or {}, pane.get("tab_id"))
        try:
            budget = _process_session(sid, tpath, pane, tries, budget, subs)
        except Exception:
            log.exception("timeline %s failed", sid[:12])
    _prune(live)


def _process_session(sid, tpath, pane, tries, budget, subs):
    data = _load(sid)
    status = pane.get("agent_status", "unknown")
    subs_sig = json.dumps(subs, sort_keys=True)
    sig = [os.path.getsize(tpath), int(os.path.getmtime(tpath)), status,
           hash(subs_sig) & 0xFFFFFFFF]
    if data.get("_sig") == sig and not _labels_missing(data):
        return budget
    data["subs"] = subs
    _parse_new(tpath, data)
    exch = data.get("_exch") or []
    working = status == "working"

    done_n = len(exch) - 1 if (exch and working) else len(exch)
    unlabeled = [ex for ex in exch[:done_n] if not ex.get("label")]
    if unlabeled and budget > 0:
        # newest first: the visible tail gets real labels before old history
        batch = unlabeled[-BATCH_LABEL_MAX:]
        try:
            labels = _label_done_batch(batch)
            budget -= 1
            if labels and len(labels) == len(batch):
                for ex, label in zip(batch, labels):
                    if label:
                        ex["label"] = label
                log.info("timeline %s labeled %d done tasks", sid[:12], len(batch))
        except Exception:
            log.exception("batch labels %s failed", sid[:12])
    if working and exch and not exch[-1].get("ylabel") and budget > 0:
        try:
            exch[-1]["ylabel"] = _label_current(exch[-1])
            budget -= 1
            log.info("timeline %s current -> %s", sid[:12], exch[-1]["ylabel"])
        except Exception:
            log.exception("current label %s failed", sid[:12])

    if exch and budget > 0:
        key = f"{exch[-1]['u']}:{len(exch[-1].get('rhead') or '')}"
        now = time.time()
        if data.get("_next_key") != key and now - tries.get(sid, 0) >= NEXT_COOLDOWN_SECS:
            tries[sid] = now
            try:
                nxt = _infer_next(
                    [ex.get("label") or "" for ex in exch[:-1] if ex.get("label")],
                    exch[-1].get("ylabel") or exch[-1].get("label") or "",
                    exch[-1].get("rhead") or "")
                budget -= 1
                if nxt is not None:
                    data["next"], data["_next_key"] = nxt, key
            except Exception:
                log.exception("next inference %s failed", sid[:12])

    data["_sig"] = sig
    _publish(data, status)
    _save(sid, data)
    return budget


def _labels_missing(data):
    exch = data.get("_exch") or []
    return any(not ex.get("label") and not ex.get("ylabel") for ex in exch)


def _prune(live):
    try:
        cutoff = time.time() - PRUNE_AGE_SECS
        for name in os.listdir(OUT_DIR):
            if not name.endswith(".json"):
                continue
            sid = name[:-5]
            full = os.path.join(OUT_DIR, name)
            if sid not in live and os.path.getmtime(full) < cutoff:
                os.remove(full)
    except FileNotFoundError:
        pass
    except Exception:
        log.exception("timeline prune failed")
