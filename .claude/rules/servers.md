---
paths:
  - "src/server.rs"
  - "src/main.rs"
  - "src/http.rs"
  - "src/chat.rs"
  - "src/answer.rs"
  - "src/conversation.rs"
  - "src/progress.rs"
  - "src/context.rs"
  - "src/tools/files.rs"
  - "src/system.rs"
  - "src/image.rs"
  - "site/index.html"
  - "README.md"
  - "src/judge.rs"
  - "src/compact.rs"
---

## LM Studio moves under Lokal

Lokal relies on LM Studio behaviors that are no API promise, and LM Studio
updates often. **When working on Lokal, compare LM Studio's version with the
one below. When it changed, check every line of this list against the running
server before anything else, then update the version and the date.** Do the
same whenever Lokal misbehaves with LM Studio.

Last checked against **LM Studio 0.4.26**, on 2026-10-14.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/v0/models` lists `id`, `type` (`llm`, `vlm`, `embeddings`), `state` (`loaded`), `loaded_context_length` and `max_context_length` | `server.rs` | `curl <host>:1234/api/v0/models` |
| A model loads with 8,192 tokens of context unless the person sets more, which is why Lokal says so at the start below 16K | `main.rs` (`SMALL_CONTEXT`) | `loaded_context_length` of a model loaded with its defaults, or `defaultContextLength` in `~/.lmstudio/settings.json`; a model's own default, set in LM Studio, wins over it |
| An unknown endpoint answers HTTP 200 with `{"error": "..."}`, so the `error` field matters more than the status | `http.rs`, `server.rs` | `curl -i <host>:1234/api/v0/nonexistent` |
| Streaming is `data: {json}` lines ending in `data: [DONE]`, and `stream_options.include_usage` adds a last piece with `usage.total_tokens`, `usage.completion_tokens` and no choices. The prompt's status counts the reply's speed from the latter, since a piece isn't always one token | `chat.rs` | a streamed `curl -N` of `/v1/chat/completions` |
| Reasoning comes in `delta.reasoning_content` (with "separate reasoning content" on) or inside `<think>` in the content | `answer.rs`, `chat.rs` | the same request, with a reasoning model |
| A tool call streams its `index`, `id` and `function.name` first, then its `arguments` in small pieces as they're generated, ending with `finish_reason: "tool_calls"`; blank content may come first. While a long file is written, the spinner shows its size growing | `chat.rs`, `conversation.rs`, `progress.rs` | a streamed request with one tool that the prompt asks it to use to write a long file |
| A `model` that doesn't exist falls back to the loaded model instead of failing | `server.rs` checks the name first | a request with `"model": "nope"` |
| `max_tokens` caps a reply, and one cut short ends with `finish_reason: "length"` | `conversation.rs` | a request with `"max_tokens": 50` and a long question |
| A tool call's `arguments` in the conversation must be valid JSON: arguments cut short get HTTP 500 with an HTML page | `context.rs` (`kept_arguments`) | a request whose history has a call with `"arguments": "{\"path\": \"a"` |
| An `image_url` part with a base64 `data:` URL in a `tool` message's content reaches a `vlm` model (tried with Qwen) | `files.rs`, `conversation.rs` | a request whose tool result holds a small PNG of two colors, asking which |
| Under WSL's NAT networking, the server is reached at the Windows host (the default gateway) only with "Serve on Local Network" on | `server.rs`, `system.rs` | `curl <gateway>:1234/v1/models` from WSL |

LM Studio writes its version to `~/.lmstudio/.internal/historical-version-info.json`
on the machine it runs on (the `targetHistories` entry whose `target` is
`lmstudio`, its `lastRecordedAppVersion`). From WSL, that's under `/mnt/c/Users/<user>/`.
Its About screen shows the same thing.

### Ollama, too

Lokal finds Ollama on its own on port 11434, after LM Studio's 1234 at
each place it looks (`server::candidates`), or takes it from `server =
localhost:11434`, and reads what it says about the model. Without `model =`,
it takes the first loaded model that uses tools, or else the first listed
that does (`server::pick_ollama_model`): Ollama lists every model pulled, an
embedding model too. Last checked against **Ollama 0.40.15**, on
2026-11-09.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/version` answers `{"version": "..."}`, which is how Lokal knows it's Ollama | `server.rs` (`is_ollama`) | `curl <host>:11434/api/version` |
| `POST /api/show` with `{"model": "<id>"}` gives `capabilities` (`vision` when the model sees, `tools` when it calls them), `model_info["<general.architecture>.context_length"]` (the most it takes) and `parameters`, a text with the model's own `num_ctx` when it has one | `server.rs` (`ollama_model`) | `curl <host>:11434/api/show -d '{"model": "<id>"}'` |
| `GET /api/ps` lists the loaded models, each `name` tagged (`:latest` when none was given), with `context_length` | `server.rs` (`loaded_context`) | `curl <host>:11434/api/ps` |
| `max_tokens` caps a reply on `/v1/chat/completions` | `conversation.rs` | a request with `"max_tokens": 40` and a long question |
| Reasoning streams in `delta.reasoning`, beside an empty `content`, never `reasoning_content` (0.40.2, 2026-11-12) | `chat.rs` (`reasoning_in`) | a streamed request to a model whose `capabilities` list `thinking` |
| A tool call comes whole once the model has written it, with nothing streaming meanwhile: 19 s for a file of 60 lines, after a sentence of text (0.40.2). So text that stops for 1.5 s is ended there, with its blank line, and the spinner shows under it, the time alone while nothing comes (`TEXT_PAUSE`) | `conversation.rs` (`ask_model`) | a streamed request with one tool that asks for a sentence, then a file written with the tool |
| An `image_url` part with a base64 `data:` URL in a `tool` message's content reaches a model with `vision` (tried with Qwen 3.8 27B and LFM3-VL 2B); a GIF gets HTTP 400 (`invalid image input`). An image adds about pixels / 1024 tokens with Qwen and with LFM3-VL, up to about 4 MP, where Ollama shrinks it | `image.rs`, `files.rs` | the same request as LM Studio's, with a PNG, a JPEG, a WebP and a GIF |

**The page's "Set up a model" tabs name each server's screens and
options:** LM Studio's Developer tab, "Serve on Local Network" and the mark
on models that use tools; Ollama's context slider, `OLLAMA_CONTEXT_LENGTH`,
`OLLAMA_HOST` and the `tools` that `ollama show` lists (from its FAQ and
context-length pages, 2026-11-11). When either changes, check those words
in `site/index.html` and `README.md` too. Both also list what Lokal won't do: a change to the
refusals changes them with it.
