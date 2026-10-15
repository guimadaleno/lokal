# Lokal — what each version brings

For whoever uses Lokal: what changed, version by version, newest first.
How versions are numbered and tagged is in `CLAUDE.md`, under "Versioning".
A version exists when it is tagged, and its entry here is what an installed
copy and the page show as its notes.

## 0.4.2 — 2026-10-14

- The model's reasoning, while it thinks, now has a blank line between it
  and the line that says it's thinking, so the two no longer run together.

## 0.4.1 — 2026-10-14

- A conversation that writes files no longer fills up with them. Once the
  model has moved on, the text of the files it wrote leaves the
  conversation, the way command outputs already did: the files hold it, and
  the model reads them again when it needs to.
- A reply that runs out of room in the middle of writing a file no longer
  breaks the conversation. Every request after it used to fail with HTTP
  500 until /new. Now that step doesn't run, the model is asked again to
  keep it short, and the conversation goes on. `lokal -r` mends a saved
  conversation that had one.

## 0.4.0 — 2026-10-13

- A long file no longer sends the model in circles when the context is
  small. Each read is cut to the room left and says where to read on, the
  model always sees what it asked for, and when the context is full it
  says so instead of asking again. A file bigger than the room left is read
  in parts, and a small model may lose track of the first ones, so for
  those, more context is what helps.
- At the start, Lokal tells you when the model is loaded with little
  context (under 16K) and could take more, and where to raise it in LM
  Studio. The model is told its context size too.
- Asking for lines past the end of a file says how many lines it has.
- While the model works, its label pulses gently, and in an approval the
  marker on the option you're on pulses a little quicker.

## 0.3.0 — 2026-10-13

- The model gets a scratch folder of the session's own for what it makes
  only to get the work done: a download, output to look through, a script to
  run once. It's private, `/info` shows where it is, and it goes when Lokal
  quits. Writing there asks first, like anywhere else.
- `/update` typed while the day's check is running no longer says it
  couldn't check, and the two can never leave a half-written Lokal behind.

## 0.2.1 — 2026-10-13

- `/help` lines up every description, `/reasoning`'s and `/feedback`'s too.
- `/about` shows the author and the website, without a link that led nowhere.

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
