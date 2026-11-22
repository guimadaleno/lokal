---
name: see
description: "Use with gms-verify before calling a change to what shows done (the prompt, the spinner, colors, layout, the words in text.rs, the page): how to record Lokal in a pseudo-terminal with a fake server, and how to look at the page."
---

# See a change

- **Lokal in a pseudo-terminal:** `lokal` in a Python `pty.fork()` at a fixed size with `TERM=xterm-256color`, its XDG folders in a temporary one with an empty `lokal/user.md` and `updates = off`, writing an asciicast, under `timeout -s INT` with the cast written in `atexit`, so a hang still leaves it. `@xterm/xterm` in a page driven by Puppeteer replays it for screenshots.
- **The driver never approves:** at *Run it?* it waits and sends Esc, then skips half a second of output, since *Run it? No* still holds the question. A turn ends after a few seconds of silence, a wait that starts again whenever keys are sent, or the checks race.
- **A scripted fake OpenAI server** (streaming `data:` pieces, tool calls picked by the last message) stands in for the model. Two sessions' sockets need a short `TMPDIR`: a Unix socket's path is short. When the real server stops answering, ask its `/v1/models` before blaming Lokal.
- **The page in a headless browser,** dark and light: `index.html#dark-still` and `#light-still` pick the theme and hold the animation. Phone width inside an `<iframe>`, since desktop Chrome won't make a window that narrow.
