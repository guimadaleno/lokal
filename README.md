# Lokal

A terminal assistant for Linux that runs on a model on your own machine.
You ask in plain words. It runs commands, reads your files and changes them,
and asks before anything changes. With a model that can see, it looks at
images too.

It talks to LM Studio, or to any server that speaks the OpenAI chat API
(Ollama works too). It finds the server by itself, also from WSL when the
server runs on Windows. It is one file of about 2 MB.

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

1. Install [LM Studio](https://lmstudio.ai) on the machine that will run the
   model.
2. Download a model that can use tools. LM Studio marks them.
3. Load it with a context of 32K tokens or more: command output and files
   fill it up.
4. In the Developer tab, switch the server on. Lokal looks for it on port
   1234.
5. On WSL, turn on "Serve on Local Network" in the server's settings, so WSL
   can reach Windows.

With another server, set its address in `~/.config/lokal/config`:

```
server = localhost:11434
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
model, asked separately, also lets commands run. A question starts on No
only for what can't be undone or leaves your machine.

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
