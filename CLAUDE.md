# Lokal

A terminal assistant for Linux on a model on the person's own machine: LM Studio, Ollama, or any server that speaks the OpenAI chat API, found by itself, also from WSL. The model runs commands, reads and changes files and calls the tools of MCP servers the person set up; Lokal asks before anything changes. One binary, few dependencies.

## Build and test

```
cargo build --release      # target/release/lokal, which never updates itself
cargo test
cargo clippy --all-targets
```

- The release build is static (musl); before touching dependencies, build it: `rustup target add x86_64-unknown-linux-musl`, then `LOKAL_RELEASE_BUILD=1 cargo build --release --target x86_64-unknown-linux-musl`. Only such a build updates itself (`update.rs`): another would undo its own rebuild.
- Try it with its files elsewhere: `XDG_CONFIG_HOME=/tmp/x XDG_STATE_HOME=/tmp/y lokal`. The first launch shows again, and the person's settings and conversation stay untouched.
- A permission feature is also tried in the owner's setup: started in `~`, files on the Windows desktop under `/mnt/c`, work split among sessions by mail. Auto passed every scripted test and still asked at every step there (#33).
- The owner runs a release build of `main`, at their request (unlike gms-handoff): after building a change for them, put it in place of their copy. It takes the next published release on its own, since only a higher version is taken.

## What it refuses to do

How each holds: the rule it names, in `.claude/rules/`.

- **Run a command that may change something without a yes.** Only `src/allowlist.rs`'s read-only list runs unasked, Lokal's code and never the model's judgement: a model can be talked into anything by a file it reads. Auto, the last mode on Shift+Tab, is the one place a model decides. Rule `commands`.
- **Write a file without showing the diff and getting a yes;** file changes have no "don't ask again". What reaches the screen from a file, a command or the model shows its control characters as marks (`layout::printable`). Rule `files`.
- **Approve on a guess where a slip costs much.** A question opens on No only for the hard core (what can't be undone, what leaves the machine, what changes what runs later), by Lokal's rule; the rest opens on Yes. Rules `commands`, `answers`.
- **Approve more than the person saw.** "Don't ask again" covers that exact command text, for the session or the project. Rule `commands`.
- **Let the model change Lokal's own files,** even with a yes: the settings, user.md, the approvals, the saved conversation, and any file one of them links to (`config::is_lokal_own`, #57). One approved diff could send every later conversation to another server, approve commands or plant instructions. The one way in is `remember`, shown as a diff. Rule `files`.
- **Take an answer from anything but a terminal as approval.** Piped stdin never says yes; without a terminal, nothing beyond reading runs; keys pressed before a question showed are discarded. Rule `answers`.
- **Let mail from another session act for the person, or run without bounds** (`src/peers.rs`): mail never approves anything, a turn mail started stays manual, and at most `mail_turns` turns in a row start on mail. Rule `mail`.
- **Let an MCP server act without a yes, or give the model its tools before the person has seen them whole** (`src/mcp.rs`, #37). Rule `mcp`.
- **Keep the model's reasoning.** It shows only in `/reasoning`'s window; the conversation and the scrollback keep none of it, and what comes inside `<think>` never shows. Rule `look`.
- **Send anything on its own beyond the update check:** one GET a day that carries nothing identifying the machine, off with `updates = off`. The model server, and an MCP server set by its address, are the only other addresses it talks to.
- **Send a report the person hasn't seen whole.** `/feedback` reaches guimadaleno/issues only when the person writes it, sees all of it and picks Send; the menu starts on Don't send. It sends kinds, never names (no server address, no model), and never the conversation, which can hold files and secrets the model read.
- **Grow on disk.** Rule `disk`.

A one-shot request (`lokal "…"`) often runs in a script: it starts on manual whatever the settings, never reaches the network on its own, gets no mail and starts no MCP server, and exits 1 unless the turn was answered (`Turn::Answered`).

## Conventions here

- **Safety before convenience.** Lokal lets a model act on a person's machine: push back on any idea that loosens the approvals, the allowlist or the refusals, and propose the safer design first (the exact command, not `cmd *`). The owner decides.
- **Simple code over clever code:** small modules with one job, short functions, plain names, no traits or generics where a function does, no abstraction before a second real use. gms-rust does not apply here.
- **Every word on screen is English, whatever the person's language, and lives in `src/text.rs`.** Rule `look`.
- **Every lasting line prints through `show!`, `showln!`, `screen::answer` or `screen::fitted`,** or is kept with `screen::kept`, never `eprintln!`: a new width prints it again. Rule `screen`.
- Examples use reserved values; what works only on WSL with LM Studio says so.
- `docs/later.md` holds what's left for later and what was decided so it isn't undone: read it before adding a feature or undoing a choice. One guards the person: the executable, never an install script.
