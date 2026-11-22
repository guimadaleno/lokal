---
paths:
  - "src/**"
  - "tests/**"
  - ".github/actions/publish/**"
  - ".github/workflows/release.yml"
  - "Cargo.toml"
---

## Invariants

- **`src/release.rs` and `tests/release/`, `src/report.rs` and
  `tests/report/` are copies, changed only at their source:**
  `guimadaleno/download` (`client/release.rs`, `testdata/`) and
  `guimadaleno/issues` (`client/report.rs`, `testdata/`). That's why both
  modules are declared with `#[rustfmt::skip]` and `#[allow(clippy::all)]`:
  an edit here makes the copies disagree.
  - **So is `.github/actions/publish/`,** `guimadaleno/download`'s
    `publish/` as of base's 86196c8, `release-v1.2.0`'s code with examples
    that name no app and the key handed to openssl through stdin, never a
    file: guimadaleno/download is private, and a public repository can't use an
    action from a private one (#42). A newer one comes in as a fresh copy,
    its commit named in `release.yml`'s comment.
- **Only the release workflow's build updates itself.** `update.rs` is
  compiled in only with `LOKAL_RELEASE_BUILD` set. Without that, a
  development build would replace itself and undo the rebuild.
- **A one-shot request (`lokal "..."`) never reaches the network on its
  own:** it often runs in a script. The update check runs in conversations
  only, and a one-shot request that got no answer exits with 1: the
  server failed, or the person said no or stopped it (anything but
  `Turn::Answered`).
- **Commands run in a session of their own** (`setsid` in `shell.rs`). Then
  nothing can wait on the terminal (sudo fails at once instead of hanging),
  and Ctrl+C reaches a command only through Lokal.
- **A change of the terminal's width lays the screen out again** (#49,
  `src/screen.rs`), as Claude Code does, but from Rust and at once.
  - **What lasts is kept as it went out:** every line of the conversation
    is printed through `show!`, `showln!`, `screen::answer` (the answer, on
    stdout) or `screen::fitted`, never `eprintln!`, and a line already on
    screen another way (the `>` line rustyline drew, a mail it printed, a
    question folded into its answer, a reply at a question) is kept with
    `screen::kept`. Text the terminal wraps by itself is kept as its
    bytes; what's fitted to the width as a layout, laid out again at the
    new width: a rule, a time at the right edge, a mail, and text broken
    to fit (`layout::print_fitted`, `layout::indented_rows`, `/info`, the
    sessions listed). A time's minute is decided once, and only its room
    again. The one exception is an MCP tool list or call shown before its
    question, whose "Show the rest" splits the lines by their count at the
    width then: it stays as it printed.
  - **What's live isn't kept:** the spinner, the draft row, the reasoning
    window, an open question and the `>` while it's typed are drawn again
    by what owns them.
  - **The redraw clears the screen and the scrollback** (`\x1b[3J`), so
    what the tab held before Lokal started goes at the first change of
    width, and prints every kept piece in one write, in a synchronized
    frame (`?2026`) where the terminal has it, with the keyboard unechoed
    meanwhile: what's typed waits for the `>`.
  - **It prints again at most 512 KB,** about a terminal's scrollback of
    text: past that the oldest go, a long answer losing its first lines
    only, and a faint line says earlier lines aren't printed again. So a
    long session redraws about as quickly as a short one, and the memory
    stays small.
  - **A clear is kept too** (`screen::cleared`): after `/new` or Ctrl+L, a
    redraw pushes what came before into the scrollback and goes on from
    the top, as the clear left it.
  - **A thread asks the terminal its width every 100 ms** and waits for
    two asks to agree, so a drag ends in one redraw. At the `>` it ends the
    reading with a SIGINT, as mail does, holding a lock the `>` must take
    to be marked left; the reading comes back with the line and the
    cursor where rustyline last drew them (`Helper::drawn`), though its
    undo and its place in the history start over. Such an ending is never
    a quit, and a signal that came as the line ended, which only raised
    Lokal's own flag, is cleared then: at the `>`, Ctrl+C is a key. Nor is
    any reading ended by a signal no one waits for (a paste's watch or
    mail that came as the line changed hands): it reads on, since a quit
    at the `>` is Ctrl+D or a second Ctrl+C, both an end of input.
  - **During a turn, the redraw is due** (`screen::due`) and happens at the
    next step of what's drawing, which then draws its live parts anew
    rather than over what's there: the spinner's tick (the model, a
    command, the judge, an MCP call, `/recon`) draws itself and the draft
    under it; the answer's loop, while it streams, the draft under the
    answer; an open question's pulse, the question, cut to the new width,
    on the option it was on. The reasoning window, which the spinner's
    redraw takes, knows from `screen::redraws` and draws again above it. A
    resize is never a key. At a reply typed at a question (`/feedback`,
    the first launch) or in an editor, it waits for the next question or
    `>`. A redraw only happens when the width really differs from the one
    laid out at, so a width changed and changed back draws nothing.
  - **A step is pinned while it runs** (`screen::pin`): what it shows,
    the `$` line, the diff, the call, never goes for the 512 KB, so a
    redraw while its question is open still shows what it asks about. A
    step that shows more (the rest of a huge diff) is kept whole until
    it's answered, at a cost of the order of what the terminal already
    took. An MCP result keeps only what can show of it.
  - **Only in a conversation whose answers and messages both reach the
    terminal:** a piped answer is never printed twice.
- **While Lokal works, it reads the keyboard itself** (`keys.rs`): Ctrl+C or
  a lone Esc stops the turn, Shift+Tab (`ESC [ Z`) changes the mode, and
  other keys go to the next message's draft (`queue.rs`, #3) rather than
  echoed over the output. The reader ends before anything asks the person
  something, so no key reaches a question it wasn't typed for.
  - **The draft shows on one row under the spinner,** drawn by the spinner
    itself (`Spinner::tick_with`), so it's there whenever the spinner is: in
    the model's turn, a command, the judge, an MCP call. While the answer's
    text streams with a draft showing, it comes a whole line at a time, the
    row on the row after (`queue::UnderAnswer`): the cursor is then always
    at a line's start, so the row is redrawn without knowing where in a
    line the answer stands. Without a draft it streams word by word, as
    before. Bracketed paste is on while the reader runs, so a pasted line
    break is text, not Enter.
  - **Enter queues it, and the model reads it after its current step,**
    as Claude Code does (`Conversation::take_queued`): a user message
    framed as sent while it worked, shown there as `> …`, and counted
    among the person's requests for auto's judge. A slash, `!` or `@` line
    waits for the turn's end, where it works as typed at the `>`.
  - **The turn's end decides where the rest goes.** A turn that ended on
    its own sends what's still queued as the next request, as typed at the
    `>`; the draft stays for the next turn. After Esc, Ctrl+C, a No or a
    failure, everything waits at the `>` as typed text (`queue::hold`):
    the person stopped to steer. ↑ on an empty draft brings what's queued
    back to edit. Mail waits behind it, as it does behind a typed line.
  - **A turn keeps the terminal unechoed from start to end** (`tty::quiet`),
    not only while a reader runs: a key typed between two steps waits for
    the next reader instead of showing in the output. Without ISIG, Ctrl+C
    is a key then too, so every wait inside a turn has a reader.
- **Mail prints only while the person is at the `>`,** through rustyline's
  `ExternalPrinter`. Outside readline that printer writes straight to the
  terminal, into a turn's output or an open editor; there, mail waits for
  the next prompt.
  - **rustyline 19 at least:** 18, with a printer, waited on the terminal
    with keys it had already read, so a paste or fast typing stalled until
    the next key.
  - **Mail ends an idle `>` with a SIGINT to Lokal itself:** readline
    returns `Interrupted` on one, and nothing else ends it from another
    thread.
    - It's sent while the inbox is held, so the person can't leave the `>`
      in between, and it's handled on the sending thread before `kill`
      returns.
    - Had they left a moment before, Lokal's own handler raises its flag
      instead, and the next turn clears that first.
  - **The status at the right says mail waits from the next key or
    prompt:** mail arriving redraws the hint as it was.
  - **The two tools and the prompt's line cost about 380 tokens a
    request,** estimated from their length; they're the same all session,
    so the prompt cache holds.
- **What the other sessions see of this one is Lokal's, never the
  model's** (`who`): its folder, the branch read from `.git/HEAD` (git
  itself runs what a repository's settings name), whether it's working or
  waiting for a yes (`Input::choose`), the person's last request cut
  short, and the files it changed.
  - **At each turn, the sessions in the same repository (or the same
    folder outside one) that came, went or moved branch** are told to the
    model as a note in that turn's request, without showing. Not in the
    system prompt: a change there would make LM Studio read the whole
    conversation again.
- **The project's notes are the project's, and take a share of the
  context** (`memory.rs`): LOKAL.md, then AGENTS.md, in the git root and
  every folder down to the working directory, each file once however it's
  reached.
  - **A note that leads outside the project is skipped:** a repository could
    link its AGENTS.md to any file of the person's, and it would reach the
    model unasked. For the same reason the walk stops at the git root: a
    file in a folder above belongs to something else.
  - **They get an eighth of the context,** cut at a line, with the model
    told to read the rest: they go along with every request, and at LM
    Studio's 8K a long AGENTS.md would leave no room to work.
  - **They come before user.md in the prompt,** framed as the
    project's: what the person wrote wins over what a repository says. The
    model changes them like any file, with a diff and a yes.
  - **They're guidance, never secrets:** they're versioned with the
    project, and other tools read them too.
  - **A change counts from the next turn, with a line saying so;**
    `/memory` opens them, and `/info` shows each one and what it costs.
- **`/recon` is Lokal's code, never the model's** (`recon.rs`): fixed
  probes that change nothing, run without a shell, each within a time
  limit. A model sent to look around would spend the context on it, and
  could be steered by what it finds.
  - **What could be a secret is left out by the code:** a remote's address
    loses its user, password and query, where a token often sits. A cloud
    tool is "configured" from whether its settings file or variable
    exists, never from what it holds. Of the other variables, only `PATH`,
    `SHELL` and `TZ` are read.
  - **Text from the disk is one short clean line,** given to the model as
    facts, not instructions: a branch or a folder name could carry some.
  - **The PATH's relative folders are skipped,** so a project can't plant a
    program the probes would run; under WSL, so are Windows' drives, whose
    programs are Windows' own and slow to look through.
  - **It lasts for the session** and shows whole on screen, just as the
    model gets it; nothing of it is written to disk.
- **One reply gets at most half the context** (`max_tokens`). A model
  reasoning in circles then stops there instead of filling all of it.
- **`/context` shows what fills the context, part by part,** as Claude
  Code's does: the system prompt's sections (`prompt::sections`, which
  join to the prompt itself), each tool, the conversation and its outputs,
  what's free and the quarter kept for replies. They're Lokal's estimates,
  on the high side, with the server's own count beside them once a reply
  has given one. `/context raw` prints the next request as it goes: each
  message's text under its role, the tools as JSON, control characters as
  marks and an image as its size. What the server's chat template adds
  around them never passes through Lokal.
- **`/compact` sums up what came before the last turn** (`compact.rs`, #4),
  and the summary takes its place. The last turn stays exact.
  - **The model gets those messages as a transcript,** as data, in a
    request of its own with no tools and reasoning off, so it sums them up
    rather than going on with the work. Outputs and arguments go in cut to
    their start; a transcript past three quarters of the context keeps its
    end.
  - **The summary goes in as a user note and an assistant reply,** so the
    roles still alternate. Never in the system prompt: it carries text from
    files the model read, and there it would weigh like instructions.
  - **All or nothing:** Esc, Ctrl+C or a failure leaves the conversation as
    it was. Text after `/compact` says what the summary should focus on.
- **An output always reaches the model before it can be dropped.** Each
  tool output is cut to the room left once the outputs the model already
  saw make way (`room_for_output`). A cut read says the line to read on
  from, and with no room at all the model is told to answer with what it
  has. Dropped unseen, the model only asks again, and each try leaves the
  history fuller, until nothing fits.
  - **Nothing it reads can fill Lokal's memory:** a command's output is
    kept as its start and end, the lines between counted (`shell.rs`), and
    a read takes a regular file only, up to 64 MB (`files::read_text`).
  - **What the person's `!` commands gave is an output too,** cut the same
    way. It goes inside their request's own message, since some chat
    templates refuse two from the user in a row, framed so `drop_own`
    finds it again to make way.
- **An image goes only to a model the server says sees** (LM Studio's
  `type: "vlm"`, Ollama's `vision` capability): one the model read goes in
  its read's own `tool` message, never as the person's words, and one the
  person pasted in their own message. A server that doesn't say gets none,
  since a model that can't see may drop the image and answer as if it had.
  - **It counts by its pixels, never its base64** (`image::tokens_in_url`),
    on the high side, and one that doesn't fit the room left, read or
    pasted, is left out with its estimate before anything older makes way
    for it: Lokal carries no decoder to shrink it. A pasted one's room is
    what's left once the request's words and the `!` outputs going with
    it are in, and the request tells the model it was left out, or it may
    answer as if it saw it.
  - **A file that reads as UTF-8 is text,** whatever its first bytes. An
    image is PNG, JPEG or WebP, up to 20 MB: Ollama refuses a GIF.
  - **It makes way like an output** once the model has replied after it,
    and the saved conversation keeps a line in its place.
  - **The person can paste one at the `>`** (Ctrl+V, or Alt+V where the
    terminal keeps Ctrl+V; #38). Windows Terminal answers Ctrl+V with an
    empty paste when the clipboard holds only an image, and rustyline
    inserts that nothing without a redraw: a paste no redraw follows
    within 300 ms stands for the image, and Lokal ends the line with a
    SIGINT, as mail does, then gives it back with the marker in
    (`input::PasteWatch`), with any model, so one that doesn't see gets
    its note as Alt+V does. Text pastes wait for nothing. Lokal reads the
    clipboard only on that key, with its own fixed program from the PATH's absolute folders
    (`powershell.exe` under WSL, `wl-paste` or `xclip` elsewhere), never
    the model, which has no tool for it. It shows as `[Image #N]`,
    numbered for the session, and goes in the person's own message, only
    while its marker is whole in the line, so what's sent is what the line
    shows. Auto's judge and the other sessions get the words only.
    - **A slow text paste isn't cut:** while rustyline's thread still
      waits in `read` or runs (from `/proc`), the paste is still coming. That
      holds because `Input` keeps a printer all session, mail or not:
      with one, rustyline waits for keys in `select`; once the last is
      dropped, it waits in `read` there too.
    - **The line given back is one being typed,** so mail waits for it,
      and keys pressed while the clipboard is read join it after the
      marker.
  - **One the server refuses gives way to a note** with the server's words,
    and the model is asked again, once (`leave_out_unseen_images`): left
    in, it would fail every request after it. Only a refusal, an error
    status or an error before any of the reply (`http::Failure::Refused`):
    a server that never answered never saw the image. A truncated PNG or an
    animated WebP reads as an image from its header, and only the server
    finds out.
- **What the model wrote into a file makes way like an output,** once it
  has replied after the write (`make_way`). The file holds that text, and
  it is most of a conversation that writes files; kept, nothing else could
  free the room. The call keeps its path, its arguments stay valid JSON
  (the chat template reads them back), and the note in their place says
  to read the file.
- **A reply that ends with nothing is never silent.** When it ran out of
  room (`finish_reason: "length"`), Lokal asks once more, with a note to
  reason briefly; the reasoning isn't kept, so the second try has room. If
  that fails too, it says the context ran out. While the model works, the
  spinner says how much of the context the reply has reached.
- **A step cut short never runs, and never stays as it came.** A reply
  that runs out of room in the middle of a tool call leaves arguments that
  aren't JSON, and LM Studio answers every later request with HTTP 500
  while they're in the conversation. They're kept as `{}`
  (`kept_arguments`), nothing that reply asked for runs, and the model
  hears why, once (`CUT_SHORT_RESULT`); a second time ends the turn.
  `lokal -r` mends a saved conversation the same way.
