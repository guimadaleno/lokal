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
  sees exactly what changes before it does, every time: file changes have no
  "don't ask again".
- **Approve on a guess.** The approval menu starts on No, so Enter alone or
  Esc never approves. "Don't ask again" covers that exact command text,
  for that session, and is never saved. A pattern like `docker *` would let
  in commands nobody saw (`docker run -v /:/host …`), and a file the model
  reads could steer it to use one. Saying no ends the turn, so the person
  says what to do instead.
- **Take an answer from anything but a terminal as approval.** Piped stdin
  never says yes; without a terminal, nothing beyond reading runs. Keys
  pressed before an approval question was on screen are discarded, because
  a "y" typed early would approve a command nobody has seen.
- **Show the model's reasoning.** It arrives separately or inside
  `<think>`, and neither reaches the screen (`src/answer.rs`).
- **Send anything on its own beyond the update check.** The model server is
  the only other address it talks to. The check is one GET a day, carries
  nothing that identifies the machine, and `updates = off` switches it off.
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
| `input.rs` | The `>` prompt (hints, completion, Ctrl+C twice) and approvals |
| `menu.rs` | The arrow-key choice the approvals use |
| `prompt.rs`, `system.rs` | The system prompt and the machine facts in it |
| `update.rs`, `install.rs` | Updating itself; `lokal --install` |
| `release.rs` | guimadaleno/release's client, **copied unchanged** |
| `config.rs` | The settings file, about-me.md, the state files |
| `spinner.rs`, `style.rs`, `interrupt.rs`, `text.rs` | Spinner, colors, Ctrl+C flag, every word on screen |
| `site/` | The page at guimadaleno.com/lokal/, published by `.github/workflows/site.yml` |
| `tools/icon.py` | Draws the mark and writes `site/`'s icons from one definition |

## Invariants

- **`src/release.rs` and `tests/release/` are copies, changed only at their
  source** (`guimadaleno/release`'s `client/release.rs` and `testdata/`).
  That's why `release.rs` is declared with `#[rustfmt::skip]` and
  `#[allow(clippy::all)]`: an edit here makes the copies disagree.
- **Only the release workflow's build updates itself.** `update.rs` is
  compiled in only with `LOKAL_RELEASE_BUILD` set. Without that, a
  development build would replace itself and undo the rebuild.
- **A one-shot request (`lokal "..."`) never reaches the network on its
  own:** it often runs in a script. The update check runs in conversations
  only.
- **Commands run in a session of their own** (`setsid` in `shell.rs`). Then
  nothing can wait on the terminal (sudo fails at once instead of hanging),
  and Ctrl+C reaches a command only through Lokal.

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
| A tool call streams its `index`, `id` and `function.name` first, then `arguments` in pieces, ending with `finish_reason: "tool_calls"`; blank content may come first | `chat.rs` | a request with one tool that the prompt asks it to use |
| A `model` that doesn't exist falls back to the loaded model instead of failing | `server.rs` checks the name first | a request with `"model": "nope"` |
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
| Release binary (static, stripped) | 1.8 MB |
| Start to the prompt, server found | ~40 ms |
| Memory at the prompt | ~3 MB |

Measured on a current laptop, under WSL.

## Unverified paths

- **Linux outside WSL:** only WSL has been run. Still owed: a plain Linux run,
  with the server on localhost.
- **Servers other than LM Studio:** the chat path should work with any
  OpenAI-compatible server through `server = host:port`, but none has been
  tried. The context gauge stays hidden without LM Studio's `/api/v0/models`.
- **Updating itself, end to end:** `update.rs` is tested piece by piece. Still
  owed: an installed 0.1.0 taking 0.1.1 from `download.guimadaleno.com`.
- **"Couldn't find a model server":** written, but not seen, since that needs
  LM Studio stopped.

## Later, not decided

- **Other local servers.** Ollama (port 11434) and llama.cpp's `llama-server`
  (8080) are the most used after LM Studio; vLLM (8000) and Jan (1337) too.
  All speak the OpenAI chat API. What differs is finding them (more ports to
  try; 8000 and 8080 are also where web servers run, so check `/v1/models`
  before trusting one) and the context length (Ollama: `/api/show`;
  llama.cpp: `/props`). Ollama is the one worth doing first.
- **A test against a real server,** run when a variable points at one
  (`LOKAL_TEST_SERVER=host:port cargo test`), turning the LM Studio table
  above into one command.
- **Publishing the page** (`site/`): it needs the deploy secrets and Lokal's
  folder on the server, and it should go out with the first release. It
  reads `latest.json` for the version and the checksum, since only
  `latest.json` and `SHA256SUMS` have version-less names; until then, the
  markup's own text is still true.
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
  the person's data (config, about-me.md, the last conversation), as the
  Windows apps do.
- **Package managers** (Homebrew, AUR, `.deb`) only when people ask. Each is
  one more channel to keep, and each updates on its own. `update.rs`
  already stays out of their way: it never replaces a copy it can't write.
- **`/info`:** model and context size, server address and how it was found,
  context in use, session time, and the files Lokal reads.

## Names that must appear

Credits the product carries (`/about`, `Cargo.toml`): the author, Guilherme
Madaleno; https://guimadaleno.com; https://github.com/guimadaleno/lokal.
