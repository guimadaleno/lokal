---
name: measure
description: "Use it before and after a change that could move Lokal's size, start time, memory, auto's judge or a redraw at a new width, before raising LM Studio's parallel or sharing one server across sessions, and when a conversation looks contaminated: what was measured, on what machine and version, and the figures the page and README quote."
---

# Measured

| What | Measured |
|---|---|
| Release binary (static, stripped) | 3.10 MB (3,101,880 bytes, as published), 3.1 MB on the page (0.12.0) |
| Start to the prompt, server found | ~40 ms (0.8.1) |
| Memory at the prompt | 2.4 MB (0.8.1) |
| Auto's judge, a 27B model with reasoning off (Ollama 0.40.2) | 0.3–2 s a step; 6–22 s with reasoning; the conversation's prompt cache stays |
| A redraw at a new width, release build, into a pseudo-terminal (#49) | 0.4 ms for 500 lines, 3 ms for 2,500, about 50 ms for the 512 KB most kept (some 7,500 lines), which the pseudo-terminal's throughput bounds; 150–220 ms from the resize, the watch's 100 ms asks included |

Measured on a current laptop, under WSL; the judge on a consumer GPU. The
page's "Light" column and the README quote the first three: a change that
moves them past what's written there changes the words too.

- **Several sessions on one LM Studio:** with `parallel = 1`, the usual
  per-model default, a second request waits for the first. With 2 slots
  each reply runs at about 35 tokens/s instead of 47 (62 together), and the
  context is **one pool shared by the requests running at once**: two that
  together pass it both fail with HTTP 400 (*Context size has been
  exceeded*), while `loaded_context_length` still shows the whole. Raising
  `parallel` needs Lokal to budget `context / parallel`, or to recover from
  that error, first.
- **A new conversation sees nothing of an old one** (checked 2026-10-19,
  after one seemed contaminated): in the code, and in LM Studio's cache with
  a hybrid 27B, by planting a random word in one request and asking for it
  in a fresh one with the same system prompt, an 11K request and a stream
  cut short included. Not tried: a session near the limit with many tool
  calls.
