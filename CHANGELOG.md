# Lokal — what each version brings

For whoever uses Lokal: what changed, version by version, newest first.
How versions are numbered and tagged is in `CLAUDE.md`, under "Versioning".
A version exists when it is tagged, and its entry here is what an installed
copy and the page show as its notes.

## 0.7.0 — 2026-11-09

- Start a line with ! to run a command yourself, like !git status. It runs
  without asking, since you typed it, and what it gives goes to the model
  with your next message.
- Shift+Tab turns on accept edits: changes to the files in your project go
  through without asking, right after you see what changes. Each session
  starts asking.
- Press Shift+Tab once more for auto. Lokal then also asks the model,
  separately, whether each everyday command is fine, and runs it without
  asking you when it says yes. It's a model on your own computer and it
  will get some wrong, so use auto at your own risk. Riskier commands, like
  deleting files or sudo, always ask. Shift+Tab again goes back to asking
  for everything.
- With a model that can see, Lokal looks at images: ask about a PNG, JPEG
  or WebP file, or drop one on the terminal to paste its path.
- With Ollama, Lokal now knows how much context the model has, so the
  prompt shows how much is in use, and a long conversation makes room as
  it does with LM Studio.
- Paste an image at the `>` with Ctrl+V (or Alt+V): a screenshot or an
  image you copied goes in as `[Image #1]`, and the model that can see
  gets it with your words. Several can go in one request.
- MCP servers: add one to your settings as `mcp.<name> = <command>` and its
  tools go to the model, once you've seen what the server says about them.
  Every call asks first. Servers that speak the older MCP and the new
  stateless one both work.

## 0.6.0 — 2026-10-29

- Lokal sessions open on the same computer can now mail each other, so
  you can split work among them. One model hands another a task and gets
  the answer back, and two sessions in the same repository hear of each
  other and agree on who works where.
- Mail shows above the > and the other session's model takes it up on its
  own, up to 64 turns in a row. Anything that changes a file or runs a
  command still asks you first. Esc stops it, and mail then waits for your
  next request in that session.
- /sessions lists your other sessions: where each works, on which branch,
  what you last asked it, and whether it's working or waiting for a yes.
- Prefer to pass each mail on yourself? Put mail = ask in /config: Enter
  passes it on, Esc drops it. mail = off turns mail off.
- Start a line with @name to mail that session yourself. /info shows each
  session's name.
- /sysprompt shows what the model is told before your first message, and
  what that costs on every request.
- In a question like "Run it?", a key such as Y now moves to its option,
  and Enter picks it, so a stray key never answers for you.
- Questions about everyday commands and changes to your project's files
  now start on Yes, so Enter approves them. Anything riskier still starts
  on No, and Esc is always No.
- You see how fast the model writes: as it works, and at the > after.
- The dot that shows Lokal is working no longer stops for seconds while
  the model thinks quickly.
- In a repository whose own git settings would run another program, git
  commands now ask first, even ones that only read.
- lokal --uninstall removes Lokal, and asks before removing your settings
  and notes.
- remember no longer saves a note twice, and can update one you already
  have.
- about-me.md is now user.md. Lokal moves yours over the first time.

## 0.5.0 — 2026-10-17

- The model's reasoning no longer shows while it thinks unless you ask for
  it: /reasoning turns its small window on for the session.
- A project's LOKAL.md and AGENTS.md now go along with every message, from
  the project's root down to the folder you're in, so the model knows how
  to work there. They take at most an eighth of the context, a line at the
  start says which go along and what they cost, /info lists them, and
  /memory opens them. A change counts from your next message.
- /recon tells the model what this machine and the project have: the
  tools on your PATH with the versions that matter, whether Docker
  answers, and the project's branch and remotes. Lokal gathers it itself,
  leaves out what could be a secret, and shows you all of what the model
  gets.
- Typing / lists the commands that fit on the line instead of wrapping,
  and /info keeps long values in their column.

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
