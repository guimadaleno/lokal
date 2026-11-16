---
paths:
  - "src/style.rs"
  - "src/screen.rs"
  - "src/layout.rs"
  - "src/markdown.rs"
  - "src/spinner.rs"
  - "src/progress.rs"
  - "src/menu.rs"
  - "src/clock.rs"
  - "src/queue.rs"
  - "src/answer.rs"
  - "src/reasoning.rs"
  - "src/diff.rs"
  - "src/input.rs"
  - "src/commands.rs"
  - "src/tools/mail.rs"
  - "src/peers/show.rs"
  - "src/main.rs"
  - "src/config.rs"
---

## The look

- **The look stays discreet:** faint and gray tones from `src/style.rs`, one
  line, and no boxes or color blocks. Orange (`style::accent`) marks the
  person's own lines and Lokal acting; code in an answer is a soft
  blue-gray. A full-width background was tried on the prompt line and
  glitched, so don't bring it back.
  - **The mode at the right of the `>` has a color of its own**
    (`style::mode`), so it's seen at a glance: accept edits in a soft
    pink, auto in a yellow that stands out more, since it asks least.
    The rest of the status stays faint.
  - **Mail a session sends reads like a mention:** `@name` in the accent
    and in italics, and the mail under it faint and italic
    (`style::mention`, `style::faint_italic`). It's the only italic
    anywhere.
  - **What's typed while Lokal works sits under the spinner** (`queue::row`):
    `>` in the accent as at the prompt, the text gray, the cursor a gray
    block, and what Enter queued faint (`queued · …`). Nothing shows until
    a key is pressed, and a long draft shows its end.
  - **Two things pulse, gently** (`style::pulse_*`, in truecolor, since the
    256-color steps are too coarse to ease): the label while the model
    works, slowly, and the marker on the option an approval is on, quicker.
    Only the marker moves, never an option itself: the pulse follows the
    selection, wherever the question opened, instead of pulling toward one.
  - **When each message was sent sits faint at the right edge** (#46,
    `src/clock.rs`), so scrolling back says when the person asked and when
    the model answered. A line sent at the `>` (a request, a `!` line,
    `@name` mail) gets it on its last row once Enter is pressed, where the
    status sat while it was typed; a queued line, when Enter queued it; the
    rule under an answer ends in when the turn ended; a mail, by its `·`
    line. Hours and minutes from the computer's clock, and the date on the
    first time of a day other than the last one shown.
    - **Only when the minute changed:** a time that would repeat the last
      one shown doesn't show, so a quick exchange carries one time and a
      turn that ran into the next minute shows it on its rule. A line
      without one is in the minute above, or had no room for it. `/new`
      and Ctrl+L at any prompt clear the screen, and the next time shows
      whatever its minute (`clock::forget`); a clear of the terminal's own
      isn't seen.
    - **The words win the room:** a row without two spaces to spare goes
      without, and a slash command gets none.
    - **On screen only:** the model, the saved conversation and the other
      sessions get nothing of it, and a one-shot request shows none.
    - **`times = off` turns them off,** for whoever finds them noise.
