---
paths:
  - "src/mcp.rs"
  - "src/mcp/**"
  - "src/tools/mcp.rs"
  - "src/config.rs"
---

## Let an MCP server act without a yes, or give the model its tools before the person has seen them whole

- **Servers are set in the person's settings only** (`mcp.<name> =
  <command line>`, optionally `mcp.<name>.tools = a, b`), never a
  project's `.mcp.json`, or a cloned repository could start programs.
  The settings are Lokal's own, so the model can't add one.
- **A server starts in the home folder, never the project's:** a
  project's `.npmrc` or `uv.toml` could change what `npx` or `uvx`
  fetches and runs. Its program comes from the PATH's absolute folders,
  as for what runs unasked, and it runs in a session of its own, ended
  with Lokal (stdin closed, then SIGTERM, then SIGKILL), a closed
  terminal or a SIGTERM to Lokal included (`client::end_with_lokal`).
  A SIGKILL to Lokal leaves them running, since nothing catches it;
  most end anyway when their stdin closes.
- **A server can't hang a turn or fill Lokal's memory:** its stdin is
  written on a thread of its own, so Esc still stops a call to one that
  stopped reading; only the answer awaited is passed on; a line is at
  most 32 MB and an image 20 MB, and its stderr is read in bounded
  pieces.
- **A tool list shows before the model gets it:** each tool's name,
  description and parameters as the JSON the model gets, control and
  hidden characters as marks, asked on No ("Offer them to the model?").
  A curated view would leave a place for words to hide. A long one shows
  its first lines and how many are left out, the rest a choice away, as
  a long diff does. Its hash is kept in the state
  folder (`mcp-seen`), and a list that changes is shown again. The list
  is fixed for the session, so the prompt cache holds.
- **Every call asks, on No** ("Call it?"), with the server, the tool and
  its arguments as the JSON the server gets: a line break inside a value
  shows as `\n`, so no value passes for another parameter. Long ones
  show their start and count the rest. "Don't ask again" covers that exact call for the
  session, kept apart from commands, so a call's text can't let a
  command run unasked. A tool's name alone would be `cmd *`. Auto never
  judges a call, mail never approves one, and `readOnlyHint` is the
  server's word, never Lokal's rule. Arguments that aren't an object
  are turned down before the question, which couldn't show them.
- **What a call gives is an output:** text cut to the room left, an
  image to a model that sees, anything else a line saying so. A server's
  words on screen, its last stderr lines included, show their control
  characters as marks.
- **Stdio and tools only:** no HTTP (TLS and OAuth, and an address Lokal
  would talk to), no sampling (a server writing to the person's model),
  no resources or prompts, and a server's `instructions` aren't passed
  on. Both eras: `initialize` first, then `server/discover` with `_meta`
  (2026-07-28) when that fails.
- **Conversations at a terminal only:** a one-shot request, or a
  conversation without a terminal, starts no server, since nothing
  there could be asked about.
