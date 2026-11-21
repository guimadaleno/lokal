---
paths:
  - ".github/**"
  - "design/**"
  - "docs/**"
  - "site/**"
  - "src/**"
  - "tests/**"
  - "tools/**"
  - "README.md"
  - "Cargo.toml"
  - "CHANGELOG.md"
---

## Map

| File | Job |
|---|---|
| `main.rs` | Arguments, start-up, the conversation loop, `!` commands and mail typed at the `>` |
| `commands.rs` | The slash commands, and what they print |
| `conversation.rs` | Messages, one turn with its tools, the saved conversation |
| `context.rs` | How much of the context the conversation takes, and what makes way when it's full |
| `compact.rs` | `/compact`: what's summed up, the request that asks for it, the pair that takes its place |
| `progress.rs` | What the spinner says while the model works: time, tokens, speed, the file being written |
| `chat.rs` | One streamed chat request on its own thread |
| `answer.rs` | What of the stream is shown: no reasoning, no blank edges |
| `markdown.rs` | Bold, inline code, headings, code blocks and links, styled as the answer streams |
| `server.rs`, `http.rs` | Finding the server and its model; plain-HTTP requests |
| `tools.rs`, `shell.rs`, `diff.rs` | The model's tools, running a command, showing a change |
| `tools/command.rs` | run_command and the person's `!` commands: who decides, what the model hears |
| `tools/files.rs` | read_file, write_file and edit_file, and how a change asks: protected places, accepted edits, auto's judge |
| `image.rs` | What an image is to the model: its kind and size from its first bytes, its data URL, what it takes of the context |
| `tools/remember.rs` | remember: a line added to user.md, or one swapped for it |
| `tools/mcp.rs` | A call to an MCP server's tool: shown, asked, and what it gave as an output |
| `mcp.rs` | The session's MCP servers: started, their tool lists shown to the person, offered, ended |
| `mcp/client.rs` | One MCP server and the JSON-RPC with it, in both eras of the protocol: a process Lokal started, or an address |
| `mcp/http.rs` | A server reached by its address: a POST a message through curl, its answer JSON or an event stream, the headers each era asks for |
| `mcp/seen.rs` | The tool lists the person has seen, a hash a server |
| `tools/mail.rs` | list_sessions and mail, shown as steps |
| `allowlist.rs` | Which commands run without asking, and which questions open on Yes |
| `allowlist/hard.rs` | The hard core: what a command's question opens on No for, and what auto's judge never decides |
| `protected.rs` | Where a change opens on No in every mode: what programs run or read as they start, keys, the person's settings |
| `gitconfig.rs` | What a repository's own git settings and hooks would have git run |
| `keys.rs` | The keyboard while Lokal works: Ctrl+C and Esc stop it, other keys go to the draft |
| `queue.rs` | What's typed while Lokal works: the draft under the spinner or the answer, the lines Enter queued, where they go when the turn ends |
| `input.rs` | The `>` prompt (hints, completion, Ctrl+C twice, an image pasted as `[Image #N]`) and approvals |
| `clipboard.rs` | The clipboard's image, read on the person's key by Lokal's own fixed programs |
| `clock.rs` | When a message was sent, as the screen shows it: the local time, and the date on a new day |
| `menu.rs` | The arrow-key choice the approvals use |
| `tty.rs` | `/dev/tty` itself: its width, raw mode, waiting for a key, dropping keys typed ahead |
| `mode.rs` | The mode Shift+Tab picks: manual, accept edits or auto |
| `judge.rs` | Auto's judge: the model, asked apart, on one command |
| `prompt.rs`, `system.rs` | The system prompt and the machine facts in it |
| `memory.rs` | The project's notes: LOKAL.md and AGENTS.md, from the git root down |
| `recon.rs` | `/recon`: what the machine and the project have, gathered by Lokal itself |
| `update.rs`, `install.rs` | Updating itself; `lokal --install` and `--uninstall` |
| `release.rs` | guimadaleno/base/release's client, **copied unchanged** |
| `report.rs` | guimadaleno/base/issues' client, **copied unchanged** |
| `feedback.rs` | `/feedback`: the form, the preview, the POST through curl |
| `approvals.rs` | Commands approved for a project |
| `reasoning.rs` | The small window the reasoning shows in while the model thinks |
| `scratch.rs` | The session's folder for temporary files |
| `screen.rs` | What the conversation printed, kept to print again at a new width; the watch on the width; `show!` and `showln!` |
| `sandbox.rs` | The box (`docs/sandbox.md`): `lokal --box` sets it up and becomes the command; `tests/sandbox.rs` tries to get out of it. Nothing runs in it yet |
| `peers.rs` | Mail between the person's sessions: this one's name and socket, listing the others, sending, news of the ones working alongside |
| `peers/inbox.rs` | The mail waiting, and when it starts a turn |
| `peers/show.rs` | What the person and the model see of mail and the other sessions |
| `peers/socket.rs` | The wire between sessions and its trust boundary: who's on the other end, who sent a mail |
| `config.rs` | The settings file, user.md, the state files |
| `layout.rs` | Text made safe and laid out for the terminal: control characters out, lines broken to fit |
| `spinner.rs`, `style.rs`, `interrupt.rs`, `text.rs` | Spinner, colors, Ctrl+C flag, every word on screen |
| `site/` | The page at guimadaleno.com/lokal/, published by `.github/workflows/site.yml` |
| `README.md` | What a person reads first in the repository: what Lokal is, setting up a model, what it won't do |
| `.github/actions/publish/` | guimadaleno/base/release's publish action, **copied unchanged**: signs the manifest and uploads the release |
| `.github/workflows/ci.yml` | Formatting, lints and tests on every push to main and every pull request |
| `tools/icon.py` | Draws the mark and writes `site/`'s icons from one definition |
