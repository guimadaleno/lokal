---
paths:
  - "src/mcp.rs"
  - "src/mcp/**"
  - "src/tools/mcp.rs"
  - "src/config.rs"
---

# MCP

A server is a program the person named, and its tool descriptions are a third party's text that goes along with every request.

## Let an MCP server act without a yes, or give the model its tools before the person has seen them whole

- **Servers are set in the person's settings only** (`mcp.<name>`, `.tools`, `.header`), never a project's `.mcp.json`, or a cloned repository could start programs. The settings are Lokal's own, so the model can't add one.
- **`lokal mcp add` writes a server's line for the person** (`src/mcp/add.rs`, #61): at a terminal only, the line shown whole and asked on No, checked by the settings' own parser before anything is written. A control character is refused, since a line break would start another setting. A scripted terminal (`yes | script -c …`) could answer it, so a model's command that runs Lokal is the hard core (rule `commands`). No header option: a token would stay in the shell's history.
- **A server starts in the home folder, never the project's:** a project's `.npmrc` or `uv.toml` could change what `npx` or `uvx` fetches and runs. Its program comes from the PATH's absolute folders, and it runs in a session of its own, ended with Lokal, a closed terminal or a SIGTERM to Lokal included (`client::end_with_lokal`); a SIGKILL to Lokal can't be caught, and most servers end anyway as their stdin closes.
- **A server can't hang a turn or fill Lokal's memory:** its stdin is written on a thread of its own, so Esc still stops a call to one that stopped reading; only the answer awaited is passed on; a line, an image and its stderr are bounded.
- **A tool list shows before the model gets it,** as the JSON the model gets, control and hidden characters as marks, asked on No: a curated view would leave a place for words to hide. A long one shows its start, the rest a choice away, as a long diff does. Its hash is kept (`mcp-seen`), a list that changes is shown again, and the list is fixed for the session, so the prompt cache holds.
- **Every call asks, on No,** with the server, the tool and its arguments as the JSON the server gets: a line break inside a value shows as `\n`, so no value passes for another parameter. "Don't ask again" covers that exact call for the session, kept apart from commands, so a call's text can't let a command run unasked; a tool's name alone would be `cmd *`. Auto never judges a call, mail never approves one, and `readOnlyHint` is the server's word, never Lokal's rule. Arguments that aren't an object are turned down before the question, which couldn't show them.
- **What a call gives is an output:** text cut to the room left, an image to a model that sees, anything else a line saying so. A server's words on screen, its stderr included, show their control characters as marks.
- **Tools only:** no sampling, no resources or prompts, and a server's `instructions` aren't passed on.
- **Conversations at a terminal only:** without one, nothing could be asked about, so no server starts.

## A server reached by its address

- **Through the system's curl** (#54, `src/mcp/http.rs`), as updates are: Lokal carries no TLS of its own.
- **The address, the headers and the body reach curl on its stdin** (`--config -`), never its command line, which other users can read; a test fails if the token shows there. `-q` keeps the person's `.curlrc` out, only http and https are spoken, a redirect is never followed (the token would go with it), and a line break in any value is refused, since it would start another option.
- **Plain `http://` only to this machine or the local network** (`config::check_address`), and never through a proxy: a token sent with it can be read on the way.
- **A server can't have Lokal start curl after curl, or hold it:** what isn't a request is bounded in time and ends with Lokal; Esc stops whatever a call waits for.
- **The token is the person's `mcp.<name>.header`,** never in the address's query. No OAuth (a browser, a local callback, tokens to keep and refresh): a server that needs it is left to a stdio bridge that signs in, started as a command.
- **Not spoken:** the GET stream, resuming a stream, and the old HTTP+SSE transport.
