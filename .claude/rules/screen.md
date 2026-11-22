---
paths:
  - "src/screen.rs"
  - "src/layout.rs"
  - "src/spinner.rs"
  - "src/reasoning.rs"
  - "src/menu.rs"
  - "src/input.rs"
  - "src/commands.rs"
  - "src/main.rs"
---

# A new width lays the screen out again (#49)

- **What lasts is kept as it went out** (`src/screen.rs`): text the terminal wraps by itself as its bytes; what's fitted to the width (a rule, a time at the right edge, a mail, text broken to fit by `layout::print_fitted` or `layout::indented_rows`) as a layout, laid out again. A line already on screen another way (the `>` line rustyline drew, a printed mail, a reply at a question) is kept with `screen::kept`. A time's minute is decided once, only its room again. An MCP tool list or call shown before its question stays as it printed, since "Show the rest" counted its lines at that width.
- **What's live isn't kept:** the spinner, the draft row, the reasoning window, an open question and the `>` while it's typed are drawn again by what owns them.
- **The redraw clears the screen and the scrollback,** so what the tab held before Lokal started goes at the first change of width, and prints every kept piece in one write, in a synchronized frame where the terminal has it, with the keyboard unechoed: what's typed waits for the `>`.
- **It prints again at most `MOST_KEPT`,** about a terminal's scrollback of text: past that the oldest go, a long answer losing its first lines only, with a faint line saying so. A long session redraws about as quickly as a short one, and the memory stays small.
- **A clear is kept too** (`screen::cleared`): after `/new` or Ctrl+L, a redraw pushes what came before into the scrollback and goes on from the top.
- **The width is watched until it settles,** so a drag ends in one redraw, and only a width that really differs from the one laid out at redraws.
- **At the `>`, the watch ends the reading with a SIGINT,** as mail does, and the line and cursor come back where rustyline drew them (`Helper::drawn`). Such an ending is never a quit, and at the `>` Ctrl+C stays a key: a quit there is Ctrl+D or a second Ctrl+C.
- **During a turn the redraw is due** (`screen::due`) and happens at the next step of what's drawing, which then draws its live parts anew: the spinner's tick and the draft under it, the streaming answer, an open question on the option it was on. The reasoning window learns of it from `screen::redraws`. A resize is never a key. At a reply typed at a question or in an editor, it waits for the next question or `>`.
- **A step is pinned while it runs** (`screen::pin`): its `$` line, diff or call never goes for the bound, so a redraw while its question is open still shows what it asks about.
- **Only in a conversation whose answers and messages both reach the terminal:** a piped answer is never printed twice.
