---
paths:
  - "src/queue.rs"
  - "src/keys.rs"
  - "src/input.rs"
  - "src/tty.rs"
  - "src/menu.rs"
  - "src/interrupt.rs"
---

## Take an answer from anything but a terminal as approval

- **Nor a key meant for a message being typed** (#3): what's typed while
  Lokal works goes to a draft in Lokal's memory (`queue.rs`), never the
  terminal's buffer, so it can't answer a question. While there's a
  draft, or a key went into one in the last second, a question takes only
  Esc and Ctrl+C for its first 600 ms (`menu::choose`'s `deaf`): an Enter
  meant to queue the draft would approve a step the person hasn't read.
