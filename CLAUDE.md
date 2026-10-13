# Lokal

A terminal assistant for Linux that runs on a model on the person's own
machine: LM Studio, or any server that speaks the OpenAI chat API. It finds
the server by itself (also from WSL, where the server runs on Windows), and
the model can run commands, read files and change them. One binary, few
dependencies.

## What it refuses to do

- **Run a command that may change something without a yes.** Only commands
  on the read-only list in `src/allowlist.rs` run on their own. That list is
  Lokal's code, never the model's judgement: a model can be talked into
  anything by a file it reads.
- **Write a file without showing the diff and getting a yes.** The person
  sees what changes before it does, every time: file changes have no
  "don't ask again".
  - **A long diff shows its first lines and how many are left out**
    (`src/diff.rs`), so a new file doesn't flood the conversation. "Show the
    rest", between Yes and No, prints them and asks again. Saying yes without
    them is the person's choice, made with the count in front of them.
- **Approve on a guess.** The approval menu starts on No, so Enter alone or
  Esc never approves. Saying no ends the turn, so the person says what to do
  instead.
- **Approve more than the person saw.** "Don't ask again" covers that exact
  command text, for the session or for the project.
  - **A pattern like `docker *` would let in commands nobody saw**
    (`docker run -v /:/host …`), and a file the model reads could steer it to
    use one.
  - **The project's approvals live in the person's own settings**
    (`~/.config/lokal/approved`), never inside the project, or a cloned
    repository could grant itself commands that run unasked.
  - **The project is the git root around the working directory,** or the
    directory itself. The file keeps its newest 500 lines.
- **Let the model change Lokal's own files,** not even with a yes. That means
  the settings, about-me.md, the approvals and the saved conversation, checked
  on the path the kernel would reach (`config::is_lokal_own`). Otherwise one
  approved diff could send every later conversation to another server,
  approve commands, or plant instructions in what every conversation starts
  with. The one way in is `remember`, which adds a line to about-me.md when
  the person asks, shown as a diff.
- **Take an answer from anything but a terminal as approval.** Piped stdin
  never says yes; without a terminal, nothing beyond reading runs. Keys
  pressed before an approval question was on screen are discarded, because
  a "y" typed early would approve a command nobody has seen.
- **Keep the model's reasoning.** While it thinks, its last few lines show
  faint in a small window that clears itself when the answer or a step
  begins (`src/reasoning.rs`; `/reasoning` hides it for the session). The
  conversation and the scrollback keep none of it. Reasoning sent inside
  `<think>` never shows (`src/answer.rs`).
- **Send anything on its own beyond the update check.** The model server is
  the only other address it talks to on its own. The check is one GET a day,
  carries nothing that identifies the machine, and `updates = off` switches it
  off.
- **Send a report the person hasn't seen whole.** `/feedback` goes to
  guimadaleno/issues only when the person writes it, sees all of it, and
  picks Send. The menu starts on Don't send. It sends kinds, never names:
  no server address, no model. **The conversation is never part of it,**
  since it can hold files and secrets the model read.
- **Grow on disk.** It keeps the last conversation only, and about-me.md is
  the person's own file.

## Conventions here

- **Everything is in English:** code, docs, commits and every word on
  screen. Conversation with the user stays in Portuguese.
- **Simple code over clever code:** small modules with one job, short
  functions, plain names, no traits or generics where a function does, no
  abstraction before a second real use. `stack-rust.md` does not apply here.
- **Every word on screen lives in `src/text.rs`.** The voice is casual but
  precise, like a colleague, with no jokes and no emoji. Approval prompts,
  "Stopped." and errors stay plain.
- **The look stays discreet:** faint and gray tones from `src/style.rs`, one
  line, and no boxes or color blocks. Orange (`style::accent`) marks the
  person's own lines and Lokal acting; code in an answer is a soft
  blue-gray. A full-width background was tried on the prompt line and
  glitched, so don't bring it back.
- **Every block gets room to breathe,** so the screen never feels
  cluttered. A blank line follows the request and the `$` line of a command.
  Output starts after one, only when there is output, so a silent command
  leaves no double gap. Each step ends with one, and a faint line closes
  each turn.
- **Markdown is styled on the screen only.** The conversation keeps what
  the model wrote, and a piped answer stays plain Markdown. A style ends
  with its line, so a stray marker can't paint the rest of the screen.

## Map

| File | Job |
|---|---|
| `main.rs` | Arguments, start-up, the conversation loop, slash commands |
| `conversation.rs` | Messages, one turn with its tools, freeing context, the saved conversation |
| `chat.rs` | One streamed chat request on its own thread |
| `answer.rs` | What of the stream is shown: no reasoning, no blank edges |
| `markdown.rs` | Bold, inline code, headings, code blocks and links, styled as the answer streams |
| `server.rs`, `http.rs` | Finding the server and its model; plain-HTTP requests |
| `tools.rs`, `shell.rs`, `diff.rs` | The model's tools, running a command, showing a change |
| `allowlist.rs` | Which commands run without asking |
| `keys.rs` | The keyboard while Lokal works: Ctrl+C and Esc stop it |
| `input.rs` | The `>` prompt (hints, completion, Ctrl+C twice) and approvals |
| `menu.rs` | The arrow-key choice the approvals use |
| `prompt.rs`, `system.rs` | The system prompt and the machine facts in it |
| `update.rs`, `install.rs` | Updating itself; `lokal --install` |
| `release.rs` | guimadaleno/release's client, **copied unchanged** |
| `report.rs` | guimadaleno/issues' client, **copied unchanged** |
| `feedback.rs` | `/feedback`: the form, the preview, the POST through curl |
| `approvals.rs` | Commands approved for a project |
| `reasoning.rs` | The small window the reasoning shows in while the model thinks |
| `config.rs` | The settings file, about-me.md, the state files |
| `spinner.rs`, `style.rs`, `interrupt.rs`, `text.rs` | Spinner, colors, Ctrl+C flag, every word on screen |
| `site/` | The page at guimadaleno.com/lokal/, published by `.github/workflows/site.yml` |
| `tools/icon.py` | Draws the mark and writes `site/`'s icons from one definition |

## Invariants

- **`src/release.rs` and `tests/release/`, `src/report.rs` and
  `tests/report/` are copies, changed only at their source:**
  `guimadaleno/release` (`client/release.rs`, `testdata/`) and
  `guimadaleno/issues` (`client/report.rs`, `testdata/`). That's why both
  modules are declared with `#[rustfmt::skip]` and `#[allow(clippy::all)]`:
  an edit here makes the copies disagree.
- **Only the release workflow's build updates itself.** `update.rs` is
  compiled in only with `LOKAL_RELEASE_BUILD` set. Without that, a
  development build would replace itself and undo the rebuild.
- **A one-shot request (`lokal "..."`) never reaches the network on its
  own:** it often runs in a script. The update check runs in conversations
  only.
- **Commands run in a session of their own** (`setsid` in `shell.rs`). Then
  nothing can wait on the terminal (sudo fails at once instead of hanging),
  and Ctrl+C reaches a command only through Lokal.
- **While Lokal works, it reads the keyboard itself** (`keys.rs`): Ctrl+C or
  a lone Esc stops the turn, and other keys are dropped rather than echoed
  over the output. The reader ends before anything asks the person
  something, so no key reaches a question it wasn't typed for.
- **One reply gets at most half the context** (`max_tokens`). A model
  reasoning in circles then stops there instead of filling all of it.
- **A reply that ends with nothing is never silent.** When it ran out of
  room (`finish_reason: "length"`), Lokal asks once more, with a note to
  reason briefly; the reasoning isn't kept, so the second try has room. If
  that fails too, it says the context ran out. While the model works, the
  spinner says how much of the context the reply has reached.

## LM Studio moves under Lokal

Lokal relies on LM Studio behaviors that are no API promise, and LM Studio
updates often. **When working on Lokal, compare LM Studio's version with the
one below. When it changed, check every line of this list against the running
server before anything else, then update the version and the date.** Do the
same whenever Lokal misbehaves with LM Studio.

Last checked against **LM Studio 0.4.18**, on 2026-10-13.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/v0/models` lists `id`, `type` (`llm`, `vlm`, `embeddings`), `state` (`loaded`) and `loaded_context_length` | `server.rs` | `curl <host>:1234/api/v0/models` |
| An unknown endpoint answers HTTP 200 with `{"error": "..."}`, so the `error` field matters more than the status | `http.rs`, `server.rs` | `curl -i <host>:1234/api/v0/nonexistent` |
| Streaming is `data: {json}` lines ending in `data: [DONE]`, and `stream_options.include_usage` adds a last piece with `usage.total_tokens` and no choices | `chat.rs` | a streamed `curl -N` of `/v1/chat/completions` |
| Reasoning comes in `delta.reasoning_content` (with "separate reasoning content" on) or inside `<think>` in the content | `answer.rs`, `chat.rs` | the same request, with a reasoning model |
| A tool call streams its `index`, `id` and `function.name` first, then its `arguments` in one piece once they're complete, ending with `finish_reason: "tool_calls"`; blank content may come first. While a long file is generated, nothing arrives, so the spinner shows only the time | `chat.rs`, `conversation.rs` | a request with one tool that the prompt asks it to use |
| A `model` that doesn't exist falls back to the loaded model instead of failing | `server.rs` checks the name first | a request with `"model": "nope"` |
| `max_tokens` caps a reply, and one cut short ends with `finish_reason: "length"` | `conversation.rs` | a request with `"max_tokens": 50` and a long question |
| Under WSL's NAT networking, the server is reached at the Windows host (the default gateway) only with "Serve on Local Network" on | `server.rs`, `system.rs` | `curl <gateway>:1234/v1/models` from WSL |

LM Studio writes its version to `~/.lmstudio/.internal/historical-version-info.json`
on the machine it runs on (`targetHistories` → `lmstudio` →
`lastRecordedAppVersion`). From WSL, that's under `/mnt/c/Users/<user>/`.
Its About screen shows the same thing.

**The page's "Set up LM Studio" steps name its screens and options** (the
Developer tab, "Serve on Local Network", the mark on models that use tools).
When LM Studio changes, check those words in `site/index.html` too.

## Build and validation

```
cargo build --release      # target/release/lokal, which never updates itself
cargo test                 # includes release.rs's vectors
cargo clippy --all-targets
```

- **Try it with its files elsewhere:** `XDG_CONFIG_HOME=/tmp/x XDG_STATE_HOME=/tmp/y lokal`.
  Then the first-launch question shows again, and the person's own config
  and last conversation stay untouched.
- **Look at a change in a real terminal.** Prompt, hints, spinner and colors
  are only right once seen; a pseudo-terminal that records the output is the
  way to check them unattended.
- **The release build is static (musl).** Before touching dependencies:
  `rustup target add x86_64-unknown-linux-musl`, then
  `LOKAL_RELEASE_BUILD=1 cargo build --release --target x86_64-unknown-linux-musl`.
- **The icons come from `python3 tools/icon.py`,** never from editing the
  files: the SVG, the PNG and every size of the `.ico` share its numbers, and
  16 px has a drawing of its own.
- **Look at the page in a headless browser,** dark and light:
  `index.html#dark-still` and `#light-still` pick the theme and hold the
  animation for the screenshot. Check phone width (390 px) inside an
  `<iframe>`, because desktop Chrome won't make a window that narrow.

## Versioning and releases

- **A release is a `v<version>` tag.** `.github/workflows/release.yml`
  builds it and publishes it to `download.guimadaleno.com/lokal/` with
  `guimadaleno/release`'s action. Read that repository's README before
  touching `update.rs`, `install.rs`, the workflow or the key.
- **Every tag reaches every installed copy, unasked,** so a tag is pushed
  only when the user says so.
- **`Cargo.toml`'s version, the tag and a `## <version> — <date>` entry in
  `CHANGELOG.md` must agree;** the workflow refuses a tag otherwise. The
  entry is the release's notes, written for the person using Lokal.
- **Bump the smallest part that fits:** patch for fixes, minor for anything
  new, while below 1.0.

## Keys and secrets

- **`update.pub` is the public half of Lokal's release key,** compiled in.
  The private half lives in the repository's `UPDATE_KEY` secret and in the
  user's offline backup, never in this repository. Losing it leaves every
  installed copy without updates.
- **`R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` are the repository's
  secrets** for the `guimadaleno-update-lokal` bucket, and are scoped to that
  bucket only.
- **The page publishes with `DEPLOY_KEY`, `DEPLOY_KNOWN_HOSTS`, `DEPLOY_USER`
  and `DEPLOY_HOST`,** like the other apps' pages: a deploy key the server
  holds to Lokal's folder only, through `rrsync -no-del`. Until they're set,
  `site.yml` does nothing.

## Measured

| What | Measured |
|---|---|
| Release binary (static, stripped) | 1.9 MB |
| Start to the prompt, server found | ~40 ms |
| Memory at the prompt | ~3 MB |

Measured on a current laptop, under WSL.

## Unverified paths

- **Ctrl+C or Esc while Lokal works, in a Windows console:** there, Ctrl+C
  never became SIGINT while the model thought or a command ran. At the `>`
  and in approvals, it worked.
  - **The difference:** at the prompt, Ctrl+C is read as a key. While
    working, it had to become SIGINT, and that path is what fails there. The
    Linux side was all correct: `isig` on, `intr = ^C`, Lokal in the
    foreground group, the handler installed.
  - **So now Lokal reads keys while it works** (`keys.rs`). The terminal is
    without ISIG, ICANON and ECHO, and a thread reads the tty, ended before
    any prompt or approval reads it. Tested in a pseudo-terminal: Ctrl+C and
    a lone Esc stop the model and a running command; an arrow key doesn't;
    typed keys don't echo.
  - **Still owed:** the same in the person's own console.
- **Linux outside WSL:** only WSL has been run. Still owed: a plain Linux run,
  with the server on localhost.
- **The reasoning window with wide characters or a resize while it shows:**
  rows are cut by character count, so double-width characters may wrap and
  leave a row behind. Tested only with a fake server's plain text.
- **Servers other than LM Studio:** the chat path should work with any
  OpenAI-compatible server through `server = host:port`, but none has been
  tried. The context gauge stays hidden without LM Studio's `/api/v0/models`.
- **"Couldn't find a model server":** written, but not seen, since that needs
  LM Studio stopped.

## Later, not decided

Tracked as issues: **other local servers** (#1, Ollama first), **an auto mode
inside a sandbox** (#2), **queued messages while Lokal works** (#3) and
**`/compact`** (#4).

- **Publishing the page** (`site/`): it needs the deploy secrets and Lokal's
  folder on the server, and it should go out with the first release. It
  reads `latest.json` for the version and the checksum, since only
  `latest.json` and `SHA256SUMS` have version-less names; until then, the
  markup's own text is still true.
- **The page's terminal, animated:** typing that shows what Lokal does,
  instead of a still picture.
- **Distribution: the executable, never an install script.** The one line
  downloads the release file and lets it install itself:
  `curl -fsSLo lokal https://download.guimadaleno.com/lokal/<version>/lokal && chmod +x lokal && ./lokal --install`.
  A `curl … | sh` script would be a second artifact outside the signed
  release: swapped on the server, it runs anything, and no signature
  notices. It would also be the installer program guimadaleno/release
  refuses. The page shows the file's SHA-256, from `SHA256SUMS`.
- **An aarch64 build** for ARM machines (a Raspberry Pi, WSL on Snapdragon
  Windows): a second target in `release.yml`, and its file on `update.rs`'s
  list of what a release may put in place.
- **`lokal --uninstall`:** removes the executable, and asks before removing
  the person's data (config, about-me.md, the approvals, the last
  conversation), as the Windows apps do.
- **Package managers** (Homebrew, AUR, `.deb`) only when people ask. Each is
  one more channel to keep, and each updates on its own. `update.rs`
  already stays out of their way: it never replaces a copy it can't write.
- **A test against a real server,** run when a variable points at one
  (`LOKAL_TEST_SERVER=host:port cargo test`), turning the LM Studio table
  above into one command.
- **To weigh:** `about-me.md` renamed `user.md`, moving an existing one.
- **To weigh:** a stronger warning in the approval menu when a command
  deletes something outside the working folder. Only a highlight, never a
  gate.
- **Settings stay `name = value`,** not JSON: they take comments, and a
  person edits them by hand. `/config` writes the file with every setting
  commented out the first time.

## Names that must appear

Credits the product carries (`/about`, the page, `Cargo.toml`): the author,
Guilherme Madaleno, and https://guimadaleno.com. **The repository's address,
https://github.com/guimadaleno/lokal, stays in `Cargo.toml` only:** the
repository is private, so a link to it on screen or on the page would lead
nowhere.
