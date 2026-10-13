# Lokal — what each version brings

For whoever uses Lokal: what changed, version by version, newest first.
How versions are numbered and tagged is in `CLAUDE.md`, under "Versioning".
A version exists when it is tagged, and its entry here is what an installed
copy and the page show as its notes.

## 0.2.0 — 2026-10-13

- While it works, the model says in one line what each step is for, and
  when a result changes its plan, so you can follow along.

## 0.1.1 — 2026-10-13

- A slash command typed at the first-launch question ("how do you like your
  answers?") runs as a command, instead of being saved as your answer. The
  question comes back the next time Lokal starts.

## 0.1.0 — 2026-10-13

- Lokal is a terminal assistant that runs on a model on your own machine:
  LM Studio, or any server that speaks the OpenAI API. It finds the server
  by itself, from WSL too.
- It runs commands, reads files and edits them. Read-only commands run right
  away; anything else, and every file change, waits for your yes, with the
  diff in front of you. A long diff shows its start, and "Show the rest"
  the remainder.
- Approvals are a menu that starts on No. "Don't ask again" covers that exact
  command, for the session or for the project, and never a pattern.
- While the model works, the spinner says what it's doing, for how long, and
  how much of the context the reply has reached. Its reasoning shows in a
  small window that clears itself (`/reasoning` hides it). Ctrl+C or Esc
  stops it, and so does Ctrl+C during a command.
- A reply that runs out of room is asked once more, and Lokal says so when
  nothing comes back.
- The model can't change Lokal's own files. When you ask it to remember
  something, it proposes a line for `about-me.md`, which you approve.
- Commands: `/new`, `/prefs`, `/config`, `/info`, `/update`, `/reasoning`,
  `/feedback` (a problem or an idea, sent only when you've seen all of it),
  `/help`, `/about`.
- `lokal "a request"` answers once and quits; `lokal -r` picks up the last
  conversation.
- `lokal --install` puts it in `~/.local/bin`, where it keeps itself up to
  date once a day. `updates = off` in `~/.config/lokal/config` turns that off.
