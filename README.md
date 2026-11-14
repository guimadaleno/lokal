# Lokal

A terminal agent for Linux in one small file, on the model on your own
machine: no account, no key, and your work stays on it. You ask in plain
words. It runs commands, reads your files and changes them,
and asks before anything changes. With a model that can see, it looks at
images too.

It talks to Ollama, LM Studio, or any server that speaks the OpenAI chat
API, and finds the first two by itself, also from WSL when the server runs
on Windows. It is one static file of about 3 MB, starts in about 40 ms and
holds under 3 MB of memory.

## Install

The current install line, with the file's SHA-256, is on
[guimadaleno.com/lokal](https://guimadaleno.com/lokal/). It downloads the
executable, which then puts itself in `~/.local/bin` and keeps itself up to
date there. `lokal --uninstall` takes it away again, and asks before
removing your settings.

To build it yourself:

```
cargo build --release      # target/release/lokal
```

A copy you build never updates itself.

## Set up a model

With [Ollama](https://ollama.com):

1. Install it on the machine that will run the model. Using WSL? It can run
   right inside it, as on any Linux.
2. `ollama pull` a model whose `ollama show` lists `tools`. Lokal needs one
   to run commands and edit files, and picks it on its own.
3. Give it a context of 32K tokens or more, with the slider in the app's
   settings or `OLLAMA_CONTEXT_LENGTH=32768` where Ollama runs: command
   output and files fill it up.
4. With Ollama on Windows and Lokal in WSL, set `OLLAMA_HOST` to
   `0.0.0.0:11434` among your Windows account's environment variables, then
   restart Ollama, so WSL can reach it.

With [LM Studio](https://lmstudio.ai):

1. Install it on the machine that will run the model.
2. Download a model that can use tools. LM Studio marks them.
3. Load it with a context of 32K tokens or more.
4. In the Developer tab, switch the server on. Lokal looks for it on port
   1234, and before Ollama's when both run.
5. On WSL, turn on "Serve on Local Network" in the server's settings, so WSL
   can reach Windows.

With another server, set its address in `~/.config/lokal/config`:

```
server = 192.0.2.10:8080
model = some-model
```

## Use

```
lokal              start a conversation
lokal <request>    ask one thing, get the answer, done
lokal -r           pick up the last conversation
```

At the `>`, `/help` lists the commands and keys. A line starting with `!`
runs a command of your own. With a model that can see, Ctrl+V (or Alt+V)
pastes the image on your clipboard as `[Image #1]`, and several can go in
one request. Shift+Tab switches between asking for
everything, accepting file edits once their diff shows, and auto, where the
model, asked separately, also lets commands run and files change outside the
working folder. A question starts on No only for what can't be undone,
leaves your machine or changes what runs later.

Resize the terminal and Lokal lays the conversation out again at the new
width, its rules, times and notes included, also mid-answer or with a
question open. To do that it clears the terminal's scrollback and prints
its own copy, the latest 512 KB (some 7,500 lines): what the tab showed
before Lokal started goes at the first change of width.

## MCP servers

An MCP server gives the model tools Lokal doesn't have. Add one a line in
`~/.config/lokal/config`, a name of your choice and the command that starts
it:

```
mcp.notes = npx -y @modelcontextprotocol/server-filesystem /home/you/notes
mcp.notes.tools = read_text_file, list_directory    # optional: only these
```

Servers start with a conversation, in your home folder, and speak MCP over
their standard input and output. Under WSL, a Windows program that serves
MCP works by its name, such as `app.exe --mcp`, since WSL puts Windows'
folders on the PATH. The first time, and whenever a server
changes what it tells the model about its tools, Lokal shows you all of it
and asks before the model gets them. Every call asks too, and `/info` shows
what each server's tools cost in context.

## What it won't do

- Run a command that may change something without your yes. Which commands
  count as read-only is Lokal's own list, never the model's call. Auto is
  the one exception: you switch it on for a session, and what can't be
  undone or leaves your machine, like deleting outside the project, sudo
  or sending data, still asks.
- Change a file without showing you the diff first.
- Call an MCP server's tool without your yes, or give the model a server's
  tools before you've seen what it says about them. A project's own
  `.mcp.json` starts nothing.
- Take a key you pressed before the question as its answer.
- Send anything anywhere on its own, except one update check a day, which
  `updates = off` turns off. `/feedback` sends a report only when you write
  it, see all of it and choose Send.
- Grow on your disk. It keeps the last conversation only.

## Made by

Guilherme Madaleno, [guimadaleno.com](https://guimadaleno.com).

## License

MIT, in [LICENSE](LICENSE).
