# Lokal — what each version brings

For whoever uses Lokal: what changed, version by version, newest first.
How versions are numbered and tagged is in `CLAUDE.md`, under "Versioning".
A version exists when it is tagged, and its entry here is what an installed
copy and the page show as its notes.

## 0.1.0 — 2026-10-13

- Lokal is a terminal assistant that runs on a model on your own machine:
  LM Studio, or any server that speaks the OpenAI API. It finds the server
  by itself, from WSL too.
- It runs commands, reads files and edits them. Read-only commands run right
  away; anything else, and every file change, waits for your yes, with the
  diff in front of you.
- `lokal "a request"` answers once and quits; `lokal -r` picks up the last
  conversation.
- `lokal --install` puts it in `~/.local/bin`, where it keeps itself up to
  date once a day. `updates = off` in `~/.config/lokal/config` turns that off.
