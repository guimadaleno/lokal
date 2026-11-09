# Lokal

A terminal assistant for Linux that runs on a model on the person's own
machine: LM Studio, or any server that speaks the OpenAI chat API. It finds
the server by itself (also from WSL, where the server runs on Windows), and
the model can run commands, read files and change them. One binary, few
dependencies.

## What it refuses to do

- **Run a command that may change something without a yes.** Only commands
  on the read-only list in `src/allowlist.rs` run on their own, and not
  with a glob where a file named like an option could write (`rg x *` with
  a `--pre=bash`), nor with an unquoted brace, which bash expands into
  words the rule never sees (`find . {-delete,}`). That list is Lokal's
  code, never the model's judgement:
  a model can be talked into anything by a file it reads.
  - **A line the person types at the `>` after a `!` is their own command,**
    and typing it was their yes, so it isn't asked about (`tools::run_own`).
    It runs as the model's commands do, in a session of its own and without
    a terminal, but without `gitconfig::UNASKED` and with the person's
    PATH, which guard only what runs unasked. What it gives goes to the
    model with the person's next request, never with a turn mail starts.
  - **Auto, the last mode on Shift+Tab, is the one place a model decides**
    (`src/judge.rs`), and the person is told, once a session, that it will
    get some wrong and is used at their own risk. It's the model, asked
    apart from the conversation, whether a command that would open on Yes
    may run without the question.
    - **It can only take away:** its yes runs the command, and its no, or a
      reply Lokal can't read as yes or no, asks the person, on No, with its
      reason. A judge wrong toward caution costs an Enter.
    - **Lokal's floor never reaches it:** a command whose question opens on
      No (`allowlist::opens_on_yes`), or git with a repository setting that
      runs a program or after a `cd`, asks as always. Read-only commands
      and "don't ask again" approvals run as they do in every mode.
    - **Nor git's hooks:** the judge sees the command, never what the
      repository has git run as it writes. So a git command it would judge
      asks instead, on No and with it named, when the repository has an
      active hook in `.git/hooks`, or sets anything beyond the plain
      settings git writes itself (`gitconfig::write_program`). The settings
      that name a program (an alias, an editor, a credential helper, a
      merge or diff tool, a submodule's `!` update…) have no fixed end, so
      what's let through is the short list that can't run anything, not a
      list of what can. The person's own global settings and hooks don't
      count.
    - **What it reads is the person's or Lokal's own:** the person's last
      three requests as they typed them, the command, the model's
      explanation, the working folder and the files written this turn. The
      model's words, the explanation and the paths, come as one short line
      each. Never the outputs or the files the model read, where text that
      steers a model comes from.
    - **What it lets through shows,** the `$` line and a faint "auto ran
      it" with its reason, and runs as anything else no one said yes to
      (`gitconfig::UNASKED`, the PATH's absolute folders).
    - **The model's own code still runs as the person:** a script it wrote
      (accepted as an edit) and then ran with `cargo test` or `python` is a
      command the judge sees, not what the script does. Only #2's box
      bounds that.
    - **A turn mail started stays manual,** as in accept edits below.
  - **git also runs programs a repository's own settings name, even to
    read:** core.fsmonitor on `git status`, filters and diff drivers on
    `git diff`, `log -p` and `blame`, `gpg.program` on `git log` (measured
    with git 2.43). Those settings are files in `.git` that anything could
    have written. So a command with git runs unasked only when the
    repository's settings, not the person's global ones, name none of them
    (`src/gitconfig.rs`). Otherwise it's asked, on No, with the setting
    named, whatever was approved before.
    - **Lokal reads only the working folder's settings,** so git after a
      `cd` or `pushd` in the same line, or with options before its command
      (`-C`, `-c`, `--git-dir`), is asked the same way
      (`allowlist::git_elsewhere`): it may reach another repository, a
      submodule included, or settings of the command's own.
  - **What runs unasked gets fsmonitor off and stays out of submodules**
    (`gitconfig::UNASKED`, through `GIT_CONFIG_PARAMETERS`), since a
    submodule's settings are its own and Lokal doesn't read them. So an
    unasked `git status` doesn't list changes inside submodules, and a git
    option about submodules is asked about. A git before 2.26 can't say
    where a setting comes from, so there every git command is asked.
  - **What runs unasked finds programs only in the PATH's absolute
    folders** (`shell::run`): with `.` or an empty entry in it, `ls` could
    be a program a project planted.
- **Write a file without showing the diff and getting a yes.** The person
  sees what changes before it does, every time: file changes have no
  "don't ask again".
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
    next step on, and auto after it includes it. Then a change whose
    question would open on Yes (`files::write_opens_on_yes`) is written once
    its diff shows, with a faint line saying so, since that rule already
    calls it day to day.
    - **Everything else asks as always:** a file outside the working and
      scratch folders, anything hidden on the way, and `remember`.
    - **A turn mail started stays manual,** since another model wrote that
      mail, and it may have been steered.
    - **Every session starts on manual,** with no setting to start
      otherwise, so the mode is chosen with eyes open each time.
    - **A long diff still shows only its start:** the person chose not to
      be asked, and inside git, `git diff` shows the rest.
    - **The mode shows at the right of the `>` and in the spinner's label**
      while it isn't manual, so Shift+Tab is seen to take, also mid-turn.
      In a turn mail started they show the mode chosen, not the one that
      applies there.
- **Approve on a guess where a slip costs much.** A question opens on Yes,
  so Enter alone approves, only for what Lokal's own rule sees as day to day
  (`allowlist::opens_on_yes`, `files::write_opens_on_yes`), never the
  model's judgement, except in the modes the person picks on Shift+Tab
  (above), where what opens on Yes may need no Enter at all. Esc is always
  No, and a key or a number only moves to its option, for Enter to pick, so
  a stray key never approves. Saying no ends the turn, so the person says
  what to do instead.
  - **On No:** anything the rule can't read whole; commands that reach past
    the person (sudo, ssh), run code Lokal can't see (a shell, xargs, awk,
    sed, npx, `deno eval`, `curl | sh`, a Windows program), delete or
    overwrite (rm, mv, dd, `git rm`, `push`, `reset`, `clean`,
    `commit --amend`, `switch -f`), change what later git commands run
    (`git config`: `git status` runs unasked), or stop things (kill,
    `docker stop`); a file outside the working and scratch
    folders, or in a hidden file or folder on the way (.git, .env, CI); and
    `remember`, which changes what every conversation starts with.
  - **The rule only picks where the question starts,** never whether there is
    one, except for a file change in accept edits, which the person picked
    (above). A command it gets wrong opens on No, as every one did before.
  - **A working folder that is the home folder or / doesn't count:** it would
    hold every file.
- **Approve more than the person saw.** "Don't ask again" covers that exact
  command text, for the session or for the project.
  - **A pattern like `docker *` would let in commands nobody saw**
    (`docker run -v /:/host …`), and a file the model reads could steer it to
    use one.
  - **The project's approvals live in the person's own settings**
    (`~/.config/lokal/approved`), never inside the project, or a cloned
    repository could grant itself commands that run unasked.
  - **The project is the git root around the working directory,** or the
    directory itself. The file keeps its newest 500 lines.
- **Let the model change Lokal's own files,** not even with a yes. That means
  the settings, user.md, the approvals and the saved conversation, checked
  on the path the kernel would reach (`config::is_lokal_own`). Otherwise one
  approved diff could send every later conversation to another server,
  approve commands, or plant instructions in what every conversation starts
  with. The one way in is `remember`, which adds a line to user.md when
  the person asks, shown as a diff.
  - **`remember` never adds a line that's already there,** and can put a
    note in place of one line and its copies (`replaces`), so notes saved
    twice can be tidied from the conversation. It can't remove a line
    without putting the new one in its place: the person sees the swap in
    the diff, and `/prefs` is where they delete.
  - **user.md was about-me.md:** a copy under the old name moves to the new
    one the first time it's needed (`config::user_md_path`), and stays in use
    where the move fails, so the person's notes are never lost.
- **Take an answer from anything but a terminal as approval.** Piped stdin
  never says yes; without a terminal, nothing beyond reading runs. Keys
  pressed before an approval question was on screen are discarded, because
  a "y" typed early would approve a command nobody has seen.
- **Let mail from another session act for the person, or run without
  bounds.** The person's Lokal sessions on one machine mail each other, so
  work can be split among them and they agree on who works where
  (`src/peers.rs`). Another model wrote the mail, and a file it read may
  have steered it.
  - **`mail = auto`, the default, lets mail start a turn on its own,** as
    in Claude Code, in a session just opened too. The `>` must sit on an
    empty line: mail waits while the person types or a turn runs.
    - `mail = ask` makes every mail wait for Enter on an empty line, and
      Esc drops it.
    - `mail = off` means no socket and no tools.
  - **Esc stops the exchange, not only the turn:** after the person stops
    a turn mail started, or says no in one, mail waits for their next
    request there. Otherwise the next mail would start a turn at once,
    and two models answering each other couldn't be stopped. Enter on an
    empty line passes waiting mail on sooner.
  - **At most `mail_turns` (64) turns in a row start on mail,** counted
    from the person's last request; past that, mail waits the same way.
    The sender's model hears so and tells its person. Without a bound, two
    models answering each other would fill the GPU and the context.
  - **The prompt and the `mail` tool steer away from loops of politeness.**
    They say to answer what asks or needs something, a plain yes or no
    included, but never mail only to thank or to say it was received.
    - A mail marked `now: false` (news, thanks) starts no turn: it goes
      along with the next one.
    - The bound is still the guard, since a prompt is only a nudge, and a
      steered model ignores it.
  - **Mail never approves anything:** every step that isn't read-only
    asks the person at that terminal, as always. A turn mail started stops
    with Esc or Ctrl+C like any other.
  - **The model gets mail as another session's words,** framed with its
    name and folder, never as the person's. The exception is mail the
    person wrote at the other session's `>` (`@name …`), which Lokal
    sends itself and frames as theirs.
  - **What another model wrote always shows:** each mail whole, without
    control characters and with the bidi controls that reorder text shown
    by their code (`<U+202E>`), and sending as a step.
    - Lokal's own facts about the other sessions go to the model without
      showing, the way its facts about the machine do: names, folders,
      branches, what each does.
    - `/sessions` shows the same facts, as `/sysprompt` and `/info` show
      the others.
  - **Only the person's own sessions talk:** a socket per session,
    `<n>.sock`, beside its scratch folder in the private
    `/tmp/lokal-<uid>`, with `n` the smallest number free.
    - The kernel says who connected (`SO_PEERCRED`): the same user, and
      the very process whose socket the sender's name points to, which is
      asked back who it is.
    - Nothing that runs unasked can open a socket, so a model can't pass
      its own words off as a session's, or as the person's.
  - **Conversations at a terminal only,** where someone sees mail arrive
    and can stop what it starts: a one-shot request neither listens nor
    gets the tools.
  - **Bounded:** at most 2,000 characters a mail and 5 waiting in a
    session; the sender's model is told to shorten it, or that it's full.
    The socket is all it puts on disk, and it goes when the session quits,
    or at the next start if the session was killed.
- **Keep the model's reasoning.** It shows only when the person asks:
  `/reasoning` turns on, for the session, a small window where its last few
  lines show faint while it thinks, and which clears itself when the answer
  or a step begins (`src/reasoning.rs`). It starts off, since a window
  redrawing on every turn is noise to whoever doesn't read it. The
  conversation and the scrollback keep none of it. Reasoning sent inside
  `<think>` never shows (`src/answer.rs`).
- **Send anything on its own beyond the update check.** The model server is
  the only other address it talks to on its own. The check is one GET a day,
  carries nothing that identifies the machine, and `updates = off` switches it
  off.
- **Send a report the person hasn't seen whole.** `/feedback` goes to
  guimadaleno/base/issues only when the person writes it, sees all of it, and
  picks Send. The menu starts on Don't send. It sends kinds, never names:
  no server address, no model. **The conversation is never part of it,**
  since it can hold files and secrets the model read.
- **Grow on disk.** It keeps the last conversation only, and user.md is
  the person's own file.
  - **The scratch folder** (`src/scratch.rs`, `/tmp/lokal-<uid>/<pid>`) is
    where the model is told to put what it makes only to get the work done.
    It goes when Lokal quits, and one left by a Lokal that was killed (a
    closed terminal) goes at the next start.
  - **It is private or it isn't there:** only a real folder of the person's
    own, closed to others, since `/tmp` is everyone's and a folder someone
    else made would let them read or swap what's written in it. Writing
    there asks first, like anywhere else.
  - **`lokal --uninstall` removes the executable, then asks before the
    person's files,** starting on keeping them (`src/install.rs`). It
    touches only what `config::own_files` names, and a folder only once
    it's empty: anything else in it isn't Lokal's.

## Conventions here

- **Everything is in English:** code, docs, commits and every word on
  screen. Conversation with the user stays in Portuguese.
- **Safety before convenience.** Lokal lets a model act on a person's
  machine, and it is meant to go public: push back on any idea that loosens
  the approvals, the allowlist or the refusals, and propose the safer design
  first (a session's *don't ask again* for the exact command, not `cmd *`).
  The user decides.
- **Nothing of one machine in the code:** no personal paths, addresses or
  names, and examples use reserved values. What works only on WSL with LM
  Studio says so.
- **Simple code over clever code:** small modules with one job, short
  functions, plain names, no traits or generics where a function does, no
  abstraction before a second real use. `stack-rust.md` does not apply here.
- **Every word on screen lives in `src/text.rs`.** The voice is casual but
  precise, like a colleague, with no jokes and no emoji. Approval prompts,
  "Stopped." and errors stay plain.
  - **The page's terminal plays the real screen** (`site/index.html`): the
    approval's options, the working labels, the prompt's status, the
    spinner's frames, the reasoning window (as `/reasoning` shows it) and
    the two pulses, at Lokal's timings. When one of them changes here, it
    changes there too. Its markup is the last frame, and what shows without
    motion.
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
  - **Two things pulse, gently** (`style::pulse_*`, in truecolor, since the
    256-color steps are too coarse to ease): the label while the model
    works, slowly, and the marker on the option an approval is on, quicker.
    Only the marker moves, never an option itself: the pulse follows the
    selection, wherever the question opened, instead of pulling toward one.
- **Every block gets room to breathe,** so the screen never feels
  cluttered. A blank line follows the request and the `$` line of a command.
  Output starts after one, only when there is output, so a silent command
  leaves no double gap. Each step ends with one, and a faint line closes
  each turn. The reasoning window keeps one between itself and the spinner.
  Mail, and the lines about it and the other sessions, get one around
  them: a faint `·`, gray, indented, and the mail itself plain.
- **Markdown is styled on the screen only.** The conversation keeps what
  the model wrote, and a piped answer stays plain Markdown. A style ends
  with its line, so a stray marker can't paint the rest of the screen.

## Map

| File | Job |
|---|---|
| `main.rs` | Arguments, start-up, the conversation loop, `!` commands and mail typed at the `>` |
| `commands.rs` | The slash commands, and what they print |
| `conversation.rs` | Messages, one turn with its tools, the saved conversation |
| `context.rs` | How much of the context the conversation takes, and what makes way when it's full |
| `progress.rs` | What the spinner says while the model works: time, tokens, speed, the file being written |
| `chat.rs` | One streamed chat request on its own thread |
| `answer.rs` | What of the stream is shown: no reasoning, no blank edges |
| `markdown.rs` | Bold, inline code, headings, code blocks and links, styled as the answer streams |
| `server.rs`, `http.rs` | Finding the server and its model; plain-HTTP requests |
| `tools.rs`, `shell.rs`, `diff.rs` | The model's tools, running a command, showing a change |
| `tools/command.rs` | run_command and the person's `!` commands: who decides, what the model hears |
| `tools/files.rs` | read_file, write_file and edit_file, and where a change may open on Yes |
| `image.rs` | What an image is to the model: its kind and size from its first bytes, its data URL, what it takes of the context |
| `tools/remember.rs` | remember: a line added to user.md, or one swapped for it |
| `tools/mail.rs` | list_sessions and mail, shown as steps |
| `allowlist.rs` | Which commands run without asking, and which questions open on Yes |
| `gitconfig.rs` | What a repository's own git settings and hooks would have git run |
| `keys.rs` | The keyboard while Lokal works: Ctrl+C and Esc stop it |
| `input.rs` | The `>` prompt (hints, completion, Ctrl+C twice) and approvals |
| `menu.rs` | The arrow-key choice the approvals use |
| `tty.rs` | `/dev/tty` itself: its width, raw mode, waiting for a key, dropping keys typed ahead |
| `mode.rs` | The mode Shift+Tab picks: manual, accept edits or auto |
| `judge.rs` | Auto's judge: the model, asked apart, on one command |
| `prompt.rs`, `system.rs` | The system prompt and the machine facts in it |
| `memory.rs` | The project's notes: LOKAL.md and AGENTS.md, from the git root down |
| `recon.rs` | `/recon`: what the machine and the project have, gathered by Lokal itself |
| `update.rs`, `install.rs` | Updating itself; `lokal --install` and `--uninstall` |
| `release.rs` | guimadaleno/base/release's client, **copied unchanged** |
| `report.rs` | guimadaleno/base/issues' client, **copied unchanged** |
| `feedback.rs` | `/feedback`: the form, the preview, the POST through curl |
| `approvals.rs` | Commands approved for a project |
| `reasoning.rs` | The small window the reasoning shows in while the model thinks |
| `scratch.rs` | The session's folder for temporary files |
| `sandbox.rs` | The box (`docs/sandbox.md`): `lokal --box` sets it up and becomes the command; `tests/sandbox.rs` tries to get out of it. Nothing runs in it yet |
| `peers.rs` | Mail between the person's sessions: this one's name and socket, listing the others, sending, news of the ones working alongside |
| `peers/inbox.rs` | The mail waiting, and when it starts a turn |
| `peers/show.rs` | What the person and the model see of mail and the other sessions |
| `peers/socket.rs` | The wire between sessions and its trust boundary: who's on the other end, who sent a mail |
| `config.rs` | The settings file, user.md, the state files |
| `layout.rs` | Text made safe and laid out for the terminal: control characters out, lines broken to fit |
| `spinner.rs`, `style.rs`, `interrupt.rs`, `text.rs` | Spinner, colors, Ctrl+C flag, every word on screen |
| `site/` | The page at guimadaleno.com/lokal/, published by `.github/workflows/site.yml` |
| `README.md` | What a person reads first in the repository: what Lokal is, setting up a model, what it won't do |
| `.github/workflows/ci.yml` | Formatting, lints and tests on every push to main and every pull request |
| `tools/icon.py` | Draws the mark and writes `site/`'s icons from one definition |

## Invariants

- **`src/release.rs` and `tests/release/`, `src/report.rs` and
  `tests/report/` are copies, changed only at their source:**
  `guimadaleno/base/release` (`client/release.rs`, `testdata/`) and
  `guimadaleno/base/issues` (`client/report.rs`, `testdata/`). That's why both
  modules are declared with `#[rustfmt::skip]` and `#[allow(clippy::all)]`:
  an edit here makes the copies disagree.
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
- **While Lokal works, it reads the keyboard itself** (`keys.rs`): Ctrl+C or
  a lone Esc stops the turn, Shift+Tab (`ESC [ Z`) changes the mode, and
  other keys are dropped rather than echoed over the output. The reader ends before anything asks the person
  something, so no key reaches a question it wasn't typed for.
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
  `type: "vlm"`, Ollama's `vision` capability), in the read's own `tool`
  message: what the model looked at, never the person's words. A server
  that doesn't say gets none, since a model that can't see may drop the
  image and answer as if it had.
  - **It counts by its pixels, never its base64** (`image::tokens_in_url`),
    on the high side, and one that doesn't fit the room left is turned down
    with its estimate: Lokal carries no decoder to shrink it.
  - **A file that reads as UTF-8 is text,** whatever its first bytes. An
    image is PNG, JPEG or WebP, up to 20 MB: Ollama refuses a GIF.
  - **It makes way like an output** once the model has replied after it,
    and the saved conversation keeps a line in its place.
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

## LM Studio moves under Lokal

Lokal relies on LM Studio behaviors that are no API promise, and LM Studio
updates often. **When working on Lokal, compare LM Studio's version with the
one below. When it changed, check every line of this list against the running
server before anything else, then update the version and the date.** Do the
same whenever Lokal misbehaves with LM Studio.

Last checked against **LM Studio 0.4.26**, on 2026-10-14.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/v0/models` lists `id`, `type` (`llm`, `vlm`, `embeddings`), `state` (`loaded`), `loaded_context_length` and `max_context_length` | `server.rs` | `curl <host>:1234/api/v0/models` |
| A model loads with 8,192 tokens of context unless the person sets more, which is why Lokal says so at the start below 16K | `main.rs` (`SMALL_CONTEXT`) | `loaded_context_length` of a model loaded with its defaults, or `defaultContextLength` in `~/.lmstudio/settings.json`; a model's own default, set in LM Studio, wins over it |
| An unknown endpoint answers HTTP 200 with `{"error": "..."}`, so the `error` field matters more than the status | `http.rs`, `server.rs` | `curl -i <host>:1234/api/v0/nonexistent` |
| Streaming is `data: {json}` lines ending in `data: [DONE]`, and `stream_options.include_usage` adds a last piece with `usage.total_tokens`, `usage.completion_tokens` and no choices. The prompt's status counts the reply's speed from the latter, since a piece isn't always one token | `chat.rs` | a streamed `curl -N` of `/v1/chat/completions` |
| Reasoning comes in `delta.reasoning_content` (with "separate reasoning content" on) or inside `<think>` in the content | `answer.rs`, `chat.rs` | the same request, with a reasoning model |
| A tool call streams its `index`, `id` and `function.name` first, then its `arguments` in small pieces as they're generated, ending with `finish_reason: "tool_calls"`; blank content may come first. While a long file is written, the spinner shows its size growing | `chat.rs`, `conversation.rs`, `progress.rs` | a streamed request with one tool that the prompt asks it to use to write a long file |
| A `model` that doesn't exist falls back to the loaded model instead of failing | `server.rs` checks the name first | a request with `"model": "nope"` |
| `max_tokens` caps a reply, and one cut short ends with `finish_reason: "length"` | `conversation.rs` | a request with `"max_tokens": 50` and a long question |
| A tool call's `arguments` in the conversation must be valid JSON: arguments cut short get HTTP 500 with an HTML page | `context.rs` (`kept_arguments`) | a request whose history has a call with `"arguments": "{\"path\": \"a"` |
| An `image_url` part with a base64 `data:` URL in a `tool` message's content reaches a `vlm` model (tried with Qwen) | `files.rs`, `conversation.rs` | a request whose tool result holds a small PNG of two colors, asking which |
| Under WSL's NAT networking, the server is reached at the Windows host (the default gateway) only with "Serve on Local Network" on | `server.rs`, `system.rs` | `curl <gateway>:1234/v1/models` from WSL |

LM Studio writes its version to `~/.lmstudio/.internal/historical-version-info.json`
on the machine it runs on (the `targetHistories` entry whose `target` is
`lmstudio`, its `lastRecordedAppVersion`). From WSL, that's under `/mnt/c/Users/<user>/`.
Its About screen shows the same thing.

### Ollama, too

With the server set to Ollama (`server = localhost:11434`), Lokal reads what
it says about the model. Last checked against **Ollama 0.40.2**, on
2026-11-09.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/version` answers `{"version": "..."}`, which is how Lokal knows it's Ollama | `server.rs` (`is_ollama`) | `curl <host>:11434/api/version` |
| `POST /api/show` with `{"model": "<id>"}` gives `capabilities` (`vision` when the model sees), `model_info["<general.architecture>.context_length"]` (the most it takes) and `parameters`, a text with the model's own `num_ctx` when it has one | `server.rs` (`ollama_model`) | `curl <host>:11434/api/show -d '{"model": "<id>"}'` |
| `GET /api/ps` lists the loaded models, each `name` tagged (`:latest` when none was given), with `context_length` | `server.rs` (`loaded_context`) | `curl <host>:11434/api/ps` |
| `max_tokens` caps a reply on `/v1/chat/completions` | `conversation.rs` | a request with `"max_tokens": 40` and a long question |
| An `image_url` part with a base64 `data:` URL in a `tool` message's content reaches a model with `vision` (tried with Qwen 3.8); a GIF gets HTTP 400 (`invalid image input`). An image adds about pixels / 1024 tokens, up to about 4 MP, where Ollama shrinks it | `image.rs`, `files.rs` | the same request as LM Studio's, with a PNG, a JPEG, a WebP and a GIF |

**The page's "Set up LM Studio" steps name its screens and options** (the
Developer tab, "Serve on Local Network", the mark on models that use tools).
When LM Studio changes, check those words in `site/index.html` and
`README.md` too. Both also list what Lokal won't do: a change to the
refusals changes them with it.

## Build and validation

```
cargo build --release      # target/release/lokal, which never updates itself
cargo test                 # includes release.rs's vectors
cargo clippy --all-targets
```

- **Try it with its files elsewhere:** `XDG_CONFIG_HOME=/tmp/x XDG_STATE_HOME=/tmp/y lokal`.
  Then the first-launch question shows again, and the person's own config
  and last conversation stay untouched.
- **Look at a change in a real terminal.** Prompt, hints, spinner and colors
  are only right once seen; a pseudo-terminal that records the output is the
  way to check them unattended.
  - **Recording:** `lokal` in a Python `pty.fork()` at a fixed size
    (100×34, `TERM=xterm-256color`), its XDG folders in a temporary one with
    an empty `lokal/about-me.md` and `updates = off`, writing an asciicast
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
- **A permission feature is tried in the user's own setup,** not only in a
  project folder: Lokal started in `~`, files on the Windows desktop under
  `/mnt/c`, work spread over several sessions by mail. Auto passed every
  scripted test and still asked at every step there (#33).
- **The user runs a release build of `main`,** at their request: after
  building a change for them, put it in place of their copy the same way.
  It takes the next published release on its own, since only a higher
  version is taken.
- **The release build is static (musl).** Before touching dependencies:
  `rustup target add x86_64-unknown-linux-musl`, then
  `LOKAL_RELEASE_BUILD=1 cargo build --release --target x86_64-unknown-linux-musl`.
- **The icons come from `python3 tools/icon.py`,** never from editing the
  files: the SVG, the PNG and every size of the `.ico` share its numbers, and
  16 px has a drawing of its own.
- **Look at the page in a headless browser,** dark and light:
  `index.html#dark-still` and `#light-still` pick the theme and hold the
  animation for the screenshot. Check phone width (390 px) inside an
  `<iframe>`, because desktop Chrome won't make a window that narrow.

## Versioning and releases

- **A release is a `v<version>` tag.** `.github/workflows/release.yml`
  builds it and publishes it to `download.guimadaleno.com/lokal/` with
  `guimadaleno/base/release`'s action. Read its README before
  touching `update.rs`, `install.rs`, the workflow or the key.
- **Every tag reaches every installed copy, unasked,** so a tag is pushed
  only when the user says so.
- **`Cargo.toml`'s version, the tag and a `## <version> — <date>` entry in
  `CHANGELOG.md` must agree;** the workflow refuses a tag otherwise. The
  entry is the release's notes, written for the person using Lokal.
- **Bump the smallest part that fits:** patch for fixes, minor for anything
  new, while below 1.0.
- **The release that ships auto changes the page:** its refusal "Run
  anything that can change your machine without your yes" stops being true.

## Keys and secrets

- **`update.pub` is the public half of Lokal's release key,** compiled in.
  The private half lives in the repository's `UPDATE_KEY` secret and in the
  user's offline backup, never in this repository. Losing it leaves every
  installed copy without updates.
- **`R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` are the repository's
  secrets** for the `guimadaleno-update-lokal` bucket, and are scoped to that
  bucket only.
- **The page publishes with the same R2 token,** like the other apps' pages:
  `site.yml` copies `site/` into `page/` in Lokal's bucket, never deleting,
  and the site's Worker serves that prefix at guimadaleno.com/lokal/. Without
  the token, `site.yml` does nothing.

## Measured

| What | Measured |
|---|---|
| Release binary (static, stripped) | 2.2 MB |
| Start to the prompt, server found | ~40 ms |
| Memory at the prompt | ~3 MB |
| Auto's judge, a 27B model with reasoning off (Ollama 0.40.2) | 1.1–1.9 s a command; 6–22 s with reasoning; the conversation's prompt cache stays |

Measured on a current laptop, under WSL; the judge on a consumer GPU.

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

## Unverified paths

- **Ctrl+C or Esc while Lokal works, in a Windows console:** there, Ctrl+C
  never became SIGINT while the model thought or a command ran. At the `>`
  and in approvals, it worked.
  - **The difference:** at the prompt, Ctrl+C is read as a key. While
    working, it had to become SIGINT, and that path is what fails there. The
    Linux side was all correct: `isig` on, `intr = ^C`, Lokal in the
    foreground group, the handler installed.
  - **So now Lokal reads keys while it works** (`keys.rs`). The terminal is
    without ISIG, ICANON and ECHO, and a thread reads the tty, ended before
    any prompt or approval reads it. Tested in a pseudo-terminal: Ctrl+C and
    a lone Esc stop the model and a running command; an arrow key doesn't;
    typed keys don't echo.
  - **Still owed:** the same in the person's own console.
- **Linux outside WSL:** only WSL has been run. Still owed: a plain Linux run,
  with the server on localhost.
- **The box (`sandbox.rs`) outside WSL:** its escape tests ran only on WSL
  with kernel 6.18. Where the box can't be set up (an older kernel, Ubuntu's
  AppArmor refusing user namespaces) they skip. CI's runner is one of those
  (writing `/proc/self/setgroups` is refused), so a green CI says nothing
  about the box. Still owed: the tests on a plain
  Ubuntu 24.04 and on a kernel just past 6.12.
- **The reasoning window with wide characters or a resize while it shows:**
  rows are cut by character count, so double-width characters may wrap and
  leave a row behind. Tested only with a fake server's plain text.
- **Images with a model family other than Qwen:** the image goes in the
  read's `tool` message, tried with Qwen only, on LM Studio and on Ollama.
  A chat template that drops an image there would leave the model
  answering blind. Still owed: a second family that sees and uses tools;
  a user message after the result is the fallback (#17).
- **Auto's judge with LM Studio:** it sends `reasoning_effort: "none"`.
  Ollama 0.40.2 honors it (1.3 s instead of 8–12 s) and keeps the
  conversation's prompt cache across the judge's request. Neither is
  checked with LM Studio. A reasoning model that ignores it runs into the
  judge's 100 tokens and asks the person after several seconds, and with
  one slot (`parallel = 1`) the judge's request may evict the cache. Still
  owed: both, on LM Studio.
- **A turn mail started stays manual in every mode:** checked by a unit
  test (`mode::for_step`) and by reading `run_tools`, not run with two
  sessions in auto.
- **Servers other than LM Studio:** the chat path should work with any
  OpenAI-compatible server through `server = host:port`. Only Ollama 0.40.2
  has been tried, under WSL: a conversation with a command step, in auto.
  The context and its gauge come from LM Studio's `/api/v0/models`, or
  from Ollama's model `num_ctx` or `/api/ps` (above); elsewhere they stay
  unknown, and so they do on Ollama for a model that isn't loaded and has
  no `num_ctx` of its own.
  The speed is timed by Lokal from the first token on. The spinner counts
  a token a piece, and so does the prompt's status where a server sends no
  `usage.completion_tokens`: tried with a fake server that sends none.
- **"Couldn't find a model server":** written, but not seen, since that needs
  LM Studio stopped.
- **Hooks in auto, the unasked PATH and control characters as marks:** run
  in a pseudo-terminal against a fake server only. A post-commit hook made
  auto ask on No, and without it the judge let the commit through; an
  answer and an output holding ESC and OSC 52 showed `^[` marks. Still
  owed: the same with a real model, and the PATH with a `.` in it.

## Later, not decided

Tracked as issues:
- **other local servers** (#1, Ollama first);
- **auto inside a sandbox**, so the judge's mistakes stay in a box the
  kernel enforces (#2). Its design is in `docs/sandbox.md`: read it before
  touching how commands run unasked;
- **queued messages while Lokal works** (#3);
- **`/compact`, and the same on its own when dropping outputs isn't
  enough** (#4);
- **an `ask` tool** (#7);
- **a highlight for deletes outside the working folder**, to weigh (#10);
- **an aarch64 build** (#11);
- **a file bigger than the context read in parts, with Lokal noting what
  matters in each** (#16). Asking the model to keep notes was tried three
  ways and failed;
- **a vision model seeing an image through `read_file`** (#17);
- **the model reloaded with enough context when LM Studio gave it the 8K
  default, with a `context` setting**, and `server.rs` moved to `/api/v1`
  with it (#19).

Not tracked: a license, not chosen yet.

Decided, and kept here so they aren't undone:

- **No `/allow <program>`:** the person vouches for the program, but the
  model picks the arguments, and one program alone (yt-dlp) has `--exec`, a
  config file read from the working folder, `-o` anywhere and self-update.
  `/allow X` amounts to `X *`, which the refusal to approve more than the
  person saw rules out. A program allowed to run unasked belongs inside the
  box (#2).
- **The page never waits for a release, nor a release for the page:**
  `site.yml` publishes `site/` whenever it changes on `main`. The page reads
  `latest.json` for the version (the install line's and the terminal
  picture's) and the checksum, since only `latest.json` and `SHA256SUMS`
  have version-less names; the markup's own text is the fallback.
- **Distribution: the executable, never an install script.** The one line
  downloads the release file and lets it install itself:
  `curl -fsSLo lokal https://download.guimadaleno.com/lokal/<version>/lokal && chmod +x lokal && ./lokal --install`.
  A `curl … | sh` script would be a second artifact outside the signed
  release: swapped on the server, it runs anything, and no signature
  notices. It would also be the installer program guimadaleno/base/release
  refuses. The page shows the file's SHA-256, from `SHA256SUMS`.
- **Package managers** (Homebrew, AUR, `.deb`) only when people ask. Each is
  one more channel to keep, and each updates on its own. `update.rs`
  already stays out of their way: it never replaces a copy it can't write.
- **Settings stay `name = value`,** not JSON: they take comments, and a
  person edits them by hand. `/config` writes the file with every setting
  commented out the first time.

## Names that must appear

Credits the product carries (`/about`, the page, `Cargo.toml`): the author,
Guilherme Madaleno, and https://guimadaleno.com. **The repository's address,
https://github.com/guimadaleno/lokal, stays in `Cargo.toml` only:** the
repository is private, so a link to it on screen or on the page would lead
nowhere.
