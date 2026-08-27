# summarizer

The daemon feeding the sidebar and detail panel: it reads each Claude
session's transcript and stamps herdr tab tokens (title, phase, subagent
header) plus a per-session task timeline the detail panel renders.

Mirrored from the private local-agent stack; `llm.py` defaults to a
localhost endpoint and reads any API key from a file outside the repo.
