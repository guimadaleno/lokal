---
name: servers
description: "Use when Lokal misbehaves with LM Studio or Ollama, after either updates, before changing how Lokal reads what they send, and before sharing one server among sessions: the server behaviors Lokal relies on, which are no API promise, and how to check each against the running server."
---

# Servers under Lokal

Check every line against the running server before anything else; fix Lokal, or the line, where one differs. Lokal finds LM Studio on 1234 before Ollama on 11434 at each place it looks (`server::candidates`).

## LM Studio

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/v0/models` lists `id`, `type` (`llm`, `vlm`, `embeddings`), `state` (`loaded`), `loaded_context_length` and `max_context_length` | `server.rs` | `curl <host>:1234/api/v0/models` |
| A model loads with a small context unless the person sets more, which is why Lokal warns below `SMALL_CONTEXT` | `main.rs` | `loaded_context_length` of a model loaded with its defaults |
| An unknown endpoint answers HTTP 200 with `{"error": "..."}`, so the `error` field matters more than the status | `http.rs`, `server.rs` | `curl -i <host>:1234/api/v0/nonexistent` |
| Streaming is `data: {json}` lines ending in `data: [DONE]`; `stream_options.include_usage` adds a last piece with `usage` and no choices, from which the reply's speed is counted, since a piece isn't always one token | `chat.rs` | a streamed `curl -N` of `/v1/chat/completions` |
| Reasoning comes in `delta.reasoning_content` or inside `<think>` in the content | `answer.rs`, `chat.rs` | the same request, with a reasoning model |
| A tool call streams its `index`, `id` and `function.name` first, then its `arguments` in small pieces, ending with `finish_reason: "tool_calls"`; blank content may come first. The spinner shows a long file's size growing from them | `chat.rs`, `conversation.rs`, `progress.rs` | a streamed request with one tool, asked to write a long file |
| A `model` that doesn't exist falls back to the loaded one instead of failing, so Lokal checks the name first | `server.rs` | a request with `"model": "nope"` |
| `max_tokens` caps a reply, and one cut short ends with `finish_reason: "length"` | `conversation.rs` | a request with a small `max_tokens` and a long question |
| A tool call's `arguments` in the history must be valid JSON: cut short, they get HTTP 500 | `context.rs` (`kept_arguments`) | a history with a call whose arguments are `{"path": "a` |
| An `image_url` part with a base64 `data:` URL in a `tool` message reaches a `vlm` model | `files.rs`, `conversation.rs` | a tool result holding a small PNG of two colors, asking which |
| Under WSL's NAT networking, the server is reached at the Windows host (the default gateway) only when it serves on the local network | `server.rs`, `system.rs` | `curl <gateway>:1234/v1/models` from WSL |

- **Its `parallel` above 1 makes the context one pool shared by the requests running at once:** two that together pass it both fail with HTTP 400, while `loaded_context_length` still shows the whole. Raising it, or sharing one server among sessions, needs Lokal to budget `context / parallel`, or to recover from that error, first.
- **A conversation that looks contaminated by an older one:** checked once and found clean, in the code and in LM Studio's cache, by planting a random word in one request and asking for it in a fresh one with the same system prompt. Not tried: a session near the limit with many tool calls.

## Ollama

Without `model =`, Lokal takes the first loaded model that uses tools, else the first listed that does (`server::pick_ollama_model`): Ollama lists every model pulled, embedding ones too.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/version` answers `{"version": "..."}`, which is how Lokal knows it's Ollama | `server.rs` (`is_ollama`) | `curl <host>:11434/api/version` |
| `POST /api/show` gives `capabilities` (`vision`, `tools`), `model_info["<general.architecture>.context_length"]` and `parameters`, a text with the model's own `num_ctx` when it has one | `server.rs` (`ollama_model`) | `curl <host>:11434/api/show -d '{"model": "<id>"}'` |
| `GET /api/ps` lists the loaded models, each `name` tagged (`:latest` when none was given), with `context_length` | `server.rs` (`loaded_context`) | `curl <host>:11434/api/ps` |
| `max_tokens` caps a reply on `/v1/chat/completions` | `conversation.rs` | a request with a small `max_tokens` and a long question |
| Reasoning streams in `delta.reasoning`, beside an empty `content`, never `reasoning_content` | `chat.rs` (`reasoning_in`) | a streamed request to a model whose `capabilities` list `thinking` |
| A tool call comes whole once written, nothing streaming meanwhile, so text that pauses for `TEXT_PAUSE` is ended there and the spinner shows under it | `conversation.rs` (`ask_model`) | a streamed request with one tool, asking for a sentence and then a file written with the tool |
| An `image_url` part with a base64 `data:` URL in a `tool` message reaches a model with `vision`; a GIF gets HTTP 400. `image.rs`'s estimate stays above what the server counts, up to where Ollama shrinks a large image | `image.rs`, `files.rs` | the same request as LM Studio's, with a PNG, a JPEG, a WebP and a GIF |

When a server's setup screens or options change, check the page and `README.md` too (rule `page`).
