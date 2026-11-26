---
paths:
  - "src/text.rs"
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
  - "site/index.html"
---

# The look and the words

## Words

- **The voice is casual but precise, like a colleague,** with no jokes and no emoji, but for the ⚠ that opens the auto warning. Approval prompts, "Stopped." and errors stay plain.
- **Short first (#45):** one line, the fact first, and a why only where it changes what the person does next. An approval question, the auto warning, the reason a question opens on No, and an error that says what to do keep theirs: cut short there, Lokal reads as offhand where it must be taken seriously, and a No without its reason looks like a bug.
- **What goes to the model** (a bracketed note in an output) is written for the model, not shortened for the screen.

## The look

- **Discreet:** faint and gray tones from `src/style.rs`, one line, no boxes or color blocks. Orange (`style::accent`) marks the person's own lines and Lokal acting; code in an answer is a soft blue-gray. A full-width background on the prompt line glitched: don't bring it back.
- **The mode at the right of the `>` has a color of its own** (`style::mode`): accept edits a soft pink, auto a yellow that stands out more, since it asks least. The rest of the status stays faint.
- **Mail reads like a mention:** `@name` in the accent and italic, the mail under it faint and italic (`style::mention`, `style::faint_italic`), the only italic anywhere.
- **The draft under the spinner** (`queue::row`): `>` in the accent, the text gray, the cursor a gray block, what Enter queued faint (`queued · …`). Nothing shows until a key is pressed, and a long draft shows its end.
- **Two things pulse, gently,** in truecolor (`style::pulse_*`), since the 256-color steps are too coarse to ease: the label while the model works, slowly, and the marker on an approval's option, quicker. Only the marker moves, following the selection, never an option.
- **Every block gets room to breathe:** a blank line follows the request and a command's `$` line; output starts after one only when there is output, so a silent command leaves no double gap; each step ends with one, and a faint line closes each turn. The reasoning window keeps one between itself and the spinner. Mail and the lines about the other sessions get one around them: a faint `·`, gray, indented, the mail itself plain.
- **Markdown is styled on the screen only:** the conversation keeps what the model wrote, and a piped answer stays plain Markdown. A style ends with its line, so a stray marker can't paint the rest of the screen.
- **The reasoning shows only when the person asks:** `/reasoning` turns on, for the session, a small window where its last lines show faint while it thinks, cleared when the answer or a step begins (`src/reasoning.rs`). It starts off: a window redrawing every turn is noise to whoever doesn't read it.

## When each message was sent (#46, `src/clock.rs`)

- **Faint at the right edge,** so scrolling back says when the person asked and when the model answered: a line sent at the `>` on its last row once Enter is pressed; a queued line when Enter queued it; the rule under an answer when the turn ended; a mail by its `·` line. Hours and minutes from the computer's clock, the date on the first time of a new day.
- **Only when the minute changed,** so a quick exchange carries one time. `/new` and Ctrl+L clear the screen, and the next time shows whatever its minute (`clock::forget`); a clear of the terminal's own isn't seen.
- **The words win the room:** a row without room to spare goes without, and a slash command gets none.
- **On screen only:** the model, the saved conversation and the other sessions get nothing of it, and a one-shot request shows none. `times = off` turns them off.

## The page

- **The page's terminal plays the real screen** (`site/index.html`): the approval's options, the working labels, the prompt's status, the spinner's frames, the reasoning window, the times beside the messages and the two pulses, at Lokal's timings. When one changes here, it changes there too. Its markup is the last frame, and what shows without motion.
