---
name: see
description: "Use it before calling a change to what shows done (the prompt, the spinner, colors, layout, the words in text.rs, the page): Lokal looked at in a real terminal, recorded in a pseudo-terminal with a scripted fake server and a driver that never approves, screenshots of it, and the page in a headless browser, dark and light."
---

# Looking at a change

- **Look at a change in a real terminal.** Prompt, hints, spinner and colors
  are only right once seen; a pseudo-terminal that records the output is the
  way to check them unattended.
  - **Recording:** `lokal` in a Python `pty.fork()` at a fixed size
    (100×34, `TERM=xterm-256color`), its XDG folders in a temporary one with
    an empty `lokal/user.md` and `updates = off`, writing an asciicast
    v2, under `timeout -s INT` with the cast written in `atexit`, so a hang
    still leaves it. `@xterm/xterm` in a page driven by Puppeteer replays it
    for screenshots.
  - **The driver never approves:** at *Run it?* it waits and sends Esc,
    then skips half a second of output, since *Run it? No* still holds the
    question. A turn ends after about 2.5 s of silence, and that wait starts
    again whenever keys are sent, or the checks race.
  - **A scripted fake OpenAI server** (streaming `data:` pieces, tool calls
    picked by the last message) stands in for the model. Two sessions'
    sockets need a short `TMPDIR`, since a Unix socket's path stops at 108
    bytes. When the real server stops answering, ask its `/v1/models`
    before blaming Lokal.
- **Look at the page in a headless browser,** dark and light:
  `index.html#dark-still` and `#light-still` pick the theme and hold the
  animation for the screenshot. Check phone width (390 px) inside an
  `<iframe>`, because desktop Chrome won't make a window that narrow.
