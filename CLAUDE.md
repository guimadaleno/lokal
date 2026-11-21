# Lokal

A terminal assistant for Linux that runs on a model on the person's own
machine: LM Studio, or any server that speaks the OpenAI chat API. It finds
the server by itself (also from WSL, where the server runs on Windows), and
the model can run commands, read files and change them, and call the tools
of MCP servers the person sets up. One binary, few dependencies.

## What it refuses to do

- **Run a command that may change something without a yes.** Only commands
  on the read-only list in `src/allowlist.rs` run on their own, and not
  with a glob where a file named like an option could write (`rg x *` with
  a `--pre=bash`), nor with an unquoted brace, which bash expands into
  words the rule never sees (`find . {-delete,}`). That list is Lokal's
  code, never the model's judgement:
  a model can be talked into anything by a file it reads. **Auto, the last
  mode on Shift+Tab, is the one place a model decides.**
  How it holds: `.claude/rules/commands.md`.
- **Write a file without showing the diff and getting a yes.** The person
  sees what changes before it does, every time: file changes have no
  "don't ask again". **What reaches the screen from a file, a command or the
  model shows its control characters as marks** (`layout::printable`).
  How it holds: `.claude/rules/files.md`.
- **Approve on a guess where a slip costs much.** A question opens on No
  only for the hard core (what can't be undone, what leaves the machine,
  and what changes what runs later), by Lokal's own rule (`allowlist::opens_on_yes`,
  `src/allowlist/hard.rs`, `files::asking`), never the model's
  judgement. Everything else opens on Yes, so Enter alone approves, and in
  the modes the person picks on Shift+Tab or in their settings
  (`.claude/rules/files.md`, `commands.md`) it may need no Enter at all. Esc is always No, and a key or a number only moves to its option,
  for Enter to pick, so a stray key never approves. Saying no ends the
  turn, so the person says what to do instead.
  How it holds: `.claude/rules/commands.md`.
- **Approve more than the person saw.** "Don't ask again" covers that exact
  command text, for the session or for the project.
  How it holds: `.claude/rules/commands.md`.
- **Let the model change Lokal's own files,** not even with a yes. That means
  the settings, user.md, the approvals and the saved conversation, checked
  on the path the kernel would reach (`config::is_lokal_own`), and the file
  any of them links to, under any name, as a dotfiles checkout makes it
  (#57). Otherwise one
  approved diff could send every later conversation to another server,
  approve commands, or plant instructions in what every conversation starts
  with. The one way in is `remember`, which adds a line to user.md when
  the person asks, shown as a diff.
  How it holds: `.claude/rules/files.md`.
- **Take an answer from anything but a terminal as approval.** Piped stdin
  never says yes; without a terminal, nothing beyond reading runs. Keys
  pressed before an approval question was on screen are discarded, because
  a "y" typed early would approve a command nobody has seen.
  How it holds: `.claude/rules/answers.md`.
- **Let mail from another session act for the person, or run without
  bounds.** The person's Lokal sessions on one machine mail each other, so
  work can be split among them and they agree on who works where
  (`src/peers.rs`). Another model wrote the mail, and a file it read may
  have steered it. **Mail never approves anything**, **a turn mail started
  stays manual**, and **at most `mail_turns` (64) turns in a row start on
  mail**.
  How it holds: `.claude/rules/mail.md`.
- **Let an MCP server act without a yes, or give the model its tools
  before the person has seen them whole** (`src/mcp.rs`, #37). A server is
  a program the person named, and its tool descriptions are a third party's
  text that goes along with every request. What a call gives back is an
  output, shown and cut as a command's is.
  How it holds: `.claude/rules/mcp.md`.
- **Keep the model's reasoning.** It shows only when the person asks:
  `/reasoning` turns on, for the session, a small window where its last few
  lines show faint while it thinks, and which clears itself when the answer
  or a step begins (`src/reasoning.rs`). It starts off, since a window
  redrawing on every turn is noise to whoever doesn't read it. The
  conversation and the scrollback keep none of it. Reasoning sent inside
  `<think>` never shows (`src/answer.rs`).
- **Send anything on its own beyond the update check.** The model server,
  and an MCP server the person set by its address, are the only other
  addresses it talks to on its own. The check is one GET a day,
  carries nothing that identifies the machine, and `updates = off` switches it
  off.
- **Send a report the person hasn't seen whole.** `/feedback` goes to
  guimadaleno/base/issues only when the person writes it, sees all of it, and
  picks Send. The menu starts on Don't send. It sends kinds, never names:
  no server address, no model. **The conversation is never part of it,**
  since it can hold files and secrets the model read.
- **Grow on disk.** It keeps the last conversation only, a line an MCP
  server for the tool list the person saw, and user.md is the person's own
  file.
  How it holds: `.claude/rules/disk.md`.

## Conventions here

- **Everything is in English:** code, docs, commits and every word on
  screen. Conversation with the user stays in Portuguese.
- **Safety before convenience.** Lokal lets a model act on a person's
  machine, and it is meant to go public: push back on any idea that loosens
  the approvals, the allowlist or the refusals, and propose the safer design
  first (a session's *don't ask again* for the exact command, not `cmd *`).
  The user decides.
- **Nothing of one machine in the code:** no personal paths, addresses or
  names, and examples use reserved values. What works only on WSL with LM
  Studio says so.
- **Simple code over clever code:** small modules with one job, short
  functions, plain names, no traits or generics where a function does, no
  abstraction before a second real use. `stack-rust.md` does not apply here.
- **Every word on screen lives in `src/text.rs`.** The voice is casual but
  precise, like a colleague, with no jokes and no emoji. Approval prompts,
  "Stopped." and errors stay plain.
  - **Short first (#45):** one line, the fact first, and a why only where
    it changes what the person does next ("Reasoning on for this
    session."). An approval question, the auto warning, the reason a
    question opens on No, and an error that says what to do keep theirs:
    cut short there, Lokal reads as offhand where it needs to be taken
    seriously, and a No without its reason looks like a bug. What goes to
    the model (a bracketed note in an output) is written for the model,
    not shortened for the screen.
  - **The page's terminal plays the real screen** (`site/index.html`): the
    approval's options, the working labels, the prompt's status, the
    spinner's frames, the reasoning window (as `/reasoning` shows it), the
    times beside the messages and the two pulses, at Lokal's timings. When one of them changes here, it
    changes there too. Its markup is the last frame, and what shows without
    motion.
- **The look stays discreet:** `.claude/rules/look.md`.
- **Every block gets room to breathe,** so the screen never feels
  cluttered. A blank line follows the request and the `$` line of a command.
  Output starts after one, only when there is output, so a silent command
  leaves no double gap. Each step ends with one, and a faint line closes
  each turn. The reasoning window keeps one between itself and the spinner.
  Mail, and the lines about it and the other sessions, get one around
  them: a faint `·`, gray, indented, and the mail itself plain.
- **Markdown is styled on the screen only.** The conversation keeps what
  the model wrote, and a piped answer stays plain Markdown. A style ends
  with its line, so a stray marker can't paint the rest of the screen.

When Lokal misbehaves with LM Studio, compare LM Studio's version first:
`.claude/rules/servers.md`.

## Build and validation

```
cargo build --release      # target/release/lokal, which never updates itself
cargo test                 # includes release.rs's vectors
cargo clippy --all-targets
```

- **Try it with its files elsewhere:** `XDG_CONFIG_HOME=/tmp/x XDG_STATE_HOME=/tmp/y lokal`.
  Then the first-launch question shows again, and the person's own config
  and last conversation stay untouched.
- **A permission feature is tried in the user's own setup,** not only in a
  project folder: Lokal started in `~`, files on the Windows desktop under
  `/mnt/c`, work spread over several sessions by mail. Auto passed every
  scripted test and still asked at every step there (#33).
- **The user runs a release build of `main`,** at their request: after
  building a change for them, put it in place of their copy the same way.
  It takes the next published release on its own, since only a higher
  version is taken, and only a `LOKAL_RELEASE_BUILD` build updates itself
  (`.claude/rules/invariants.md`).
- **The release build is static (musl).** Before touching dependencies:
  `rustup target add x86_64-unknown-linux-musl`, then
  `LOKAL_RELEASE_BUILD=1 cargo build --release --target x86_64-unknown-linux-musl`.
- **The icons come from `python3 tools/icon.py`,** never from editing the
  files: the SVG, the PNG and every size of the `.ico` share its numbers, and
  16 px has a drawing of its own.

## Later, not decided

What is tracked for later, and what was decided and is kept so it isn't
undone: `docs/later.md`. Read it before adding a feature or undoing a choice.
One of them guards the person: **Distribution: the executable, never an
install script.**

## Names that must appear

Credits the product carries (`/about`, the page, `Cargo.toml`): the author,
Guilherme Madaleno, and https://guimadaleno.com. **The repository,
https://github.com/guimadaleno/lokal, is public:** `Cargo.toml` names it, and
the page links to it under the install line and in the footer.
