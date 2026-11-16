---
paths:
  - "src/diff.rs"
  - "src/layout.rs"
  - "src/mode.rs"
  - "src/protected.rs"
  - "src/tools/files.rs"
  - "src/tools/remember.rs"
  - "src/config.rs"
  - "src/memory.rs"
  - "src/commands.rs"
  - "src/answer.rs"
  - "src/markdown.rs"
  - "src/tools/command.rs"
  - "src/shell.rs"
  - "src/tools/mail.rs"
  - "src/tools/mcp.rs"
  - "src/peers/show.rs"
  - "src/input.rs"
  - "src/keys.rs"
---

## Write a file without showing the diff and getting a yes

- **A long diff shows its first lines and how many are left out**
  (`src/diff.rs`), so a new file doesn't flood the conversation. "Show the
  rest", between Yes and No, prints them and asks again. Saying yes without
  them is the person's choice, made with the count in front of them.
- **What reaches the screen from a file, a command or the model shows
  its control characters as marks** (`layout::printable`: ESC as `^[`, a
  lone `\r` as `^M`): the diff, the `$` line and its explanation, the
  paths, a command's output and the answer. Otherwise they could move the
  cursor and write over a line, so the diff or the command the person
  approves isn't what runs, or set the clipboard. The conversation and
  the files keep the bytes as they are.
- **Accept edits is the one exception, and the person picks it** with
  Shift+Tab (`src/mode.rs`), at the `>` or while Lokal works, from the
  next step on, and auto after it includes it. Then a change in the
  working or scratch folder, with nothing hidden on the way and in no
  protected place (`files::asking`), is written once its diff shows, with
  a faint line saying so.
  - **Auto adds the judge for the rest** (#36): a change anywhere else
    that isn't protected, a working folder of `~` and the Windows desktop
    under `/mnt/c` included, is written once the judge says yes. It reads
    the path, never the content.
  - **Everything else asks as always:** a protected place
    (`protected::is_protected`), on No, and `remember`.
  - **A turn mail started stays manual,** since another model wrote that
    mail, and it may have been steered.
  - **Every session starts on manual,** with no setting to start
    otherwise, so the mode is chosen with eyes open each time.
  - **A long diff still shows only its start:** the person chose not to
    be asked, and inside git, `git diff` shows the rest.
  - **The mode shows at the right of the `>` and in the spinner's label**
    while it isn't manual, so Shift+Tab is seen to take, also mid-turn.
    At the `>`, "(Shift+Tab to cycle)" follows it while there's room. In
    a turn mail started they show the mode chosen, not the one that
    applies there.

## Let the model change Lokal's own files

- **`remember` never adds a line that's already there,** and can put a
  note in place of one line and its copies (`replaces`), so notes saved
  twice can be tidied from the conversation. It can't remove a line
  without putting the new one in its place: the person sees the swap in
  the diff, and `/prefs` is where they delete.
- **user.md was about-me.md:** a copy under the old name moves to the new
  one the first time it's needed (`config::user_md_path`), and stays in use
  where the move fails, so the person's notes are never lost.
