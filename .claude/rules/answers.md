---
paths:
  - "src/queue.rs"
  - "src/keys.rs"
  - "src/input.rs"
  - "src/tty.rs"
  - "src/menu.rs"
  - "src/interrupt.rs"
  - "src/clipboard.rs"
  - "src/spinner.rs"
  - "src/conversation.rs"
---

# Keys and answers

## Take an answer from anything but a terminal as approval

- **Nor a key meant for a message being typed** (#3): what's typed while Lokal works goes to a draft in Lokal's memory (`queue.rs`), never the terminal's buffer, so it can't answer a question. While there's a draft, or a key went into one just before, a question takes only Esc and Ctrl+C at first (`menu::choose`'s `deaf`): an Enter meant to queue the draft would approve a step the person hasn't read.
- **Nor keys pressed before it showed, at any question** (`tty::discard_typed_ahead`): a conversation's, `lokal mcp add`'s and `--uninstall`'s (#62).
- **Nor keys a command types into the terminal:** what types keys (`tmux send-keys`, SendKeys, `xdotool`) is the hard core (rule `commands`, #62).
- **Esc is always No, and a key or a number only moves to its option,** for Enter to pick, so a stray key never approves. Saying no ends the turn, so the person says what to do instead.

## While Lokal works

- **It reads the keyboard itself** (`keys.rs`): Ctrl+C or a lone Esc stops the turn, Shift+Tab changes the mode, and other keys go to the draft rather than echoed over the output. The reader ends before anything asks the person, so no key reaches a question it wasn't typed for.
- **A turn keeps the terminal unechoed from start to end** (`tty::quiet`): a key typed between two steps waits for the next reader. Without ISIG, Ctrl+C is a key then too, so every wait inside a turn has a reader.
- **The draft shows on one row under the spinner, drawn by the spinner** (`Spinner::tick_with`), so it's there whenever the spinner is. While an answer streams with a draft showing, it comes a whole line at a time, the row under it (`queue::UnderAnswer`). Bracketed paste is on while the reader runs, so a pasted line break is text.
- **Enter queues it, and the model reads it after its current step** (`Conversation::take_queued`): framed as sent while it worked, shown as `> …`, and counted among the person's requests for auto's judge. A slash, `!` or `@` line waits for the turn's end, where it works as typed at the `>`.
- **The turn's end decides where the rest goes:** a turn that ended on its own sends what's queued as the next request; after Esc, Ctrl+C, a No or a failure, everything waits at the `>` as typed text (`queue::hold`), since the person stopped to steer. Mail waits behind it.

## An image pasted at the `>` (#38)

- **Ctrl+V, or Alt+V where the terminal keeps Ctrl+V.** Windows Terminal answers Ctrl+V with an empty paste when the clipboard holds only an image, so a paste no redraw soon follows stands for the image (`input::PasteWatch`), with any model; one that doesn't see gets its note. Text pastes wait for nothing.
- **Lokal reads the clipboard only on that key, with its own fixed program from the PATH's absolute folders** (`src/clipboard.rs`), never the model, which has no tool for it.
- **It shows as `[Image #N]` and goes only while its marker is whole in the line,** so what's sent is what the line shows. Auto's judge and the other sessions get the words only.
- **A slow text paste isn't cut:** rustyline's thread in `read` (from `/proc`) means it's still coming. That tells only because `Input` keeps a printer all session, mail or not: with one, an idle rustyline waits in `select` instead.
- **The line given back is one being typed,** so mail waits for it, and keys pressed while the clipboard is read join it after the marker.
