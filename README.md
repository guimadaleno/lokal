# Lokal

A terminal assistant for Linux that runs on a model on your own machine.
You ask in plain words. It runs commands, reads your files and changes them,
and asks before anything changes.

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
runs a command of your own. Shift+Tab switches between asking for
everything, accepting file edits once their diff shows, and auto, where the
model, asked separately, also lets everyday commands run.

## What it won't do

- Run a command that may change something without your yes. Which commands
  count as read-only is Lokal's own list, never the model's call. Auto is
  the one exception: you switch it on for a session, and anything riskier,
  like deleting files or sudo, still asks.
- Change a file without showing you the diff first.
- Take a key you pressed before the question as its answer.
- Send anything anywhere on its own, except one update check a day, which
  `updates = off` turns off. `/feedback` sends a report only when you write
  it, see all of it and choose Send.
- Grow on your disk. It keeps the last conversation only.

## Made by

Guilherme Madaleno, [guimadaleno.com](https://guimadaleno.com).
