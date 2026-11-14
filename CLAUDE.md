# Lokal

A terminal assistant for Linux that runs on a model on the person's own
machine: LM Studio, or any server that speaks the OpenAI chat API. It finds
the server by itself (also from WSL, where the server runs on Windows), and
the model can run commands, read files and change them, and call the tools
of MCP servers the person sets up. One binary, few dependencies.

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
    (`src/judge.rs`), and the person is told, once a session and as they
    pick it (`mode::show_warning`), that it will get some wrong and is used
    at their own risk: at the `>`, the line makes way for the warning and
    comes back under it; while Lokal works, it shows as soon as nothing is
    mid-line, and before a step it would decide at the latest. It's the
    model, asked apart from the conversation, whether a command that would
    open on Yes may run without the question, and a file change outside
    the working and scratch folders be written (below).
    - **It can only take away:** its yes runs the command or writes the
      file, and its no, or a reply Lokal can't read as yes or no, asks the
      person, on No, with its reason. A judge wrong toward caution costs an
      Enter.
    - **Lokal's floor never reaches it:** the hard core, whose question
      opens on No (`allowlist::opens_on_yes`, below), or git with a
      repository setting that runs a program or after a `cd` Lokal can't
      follow, asks as always. Read-only commands and "don't ask again"
      approvals run as they do in every mode.
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
      three requests as they typed them, the command, or a file change's
      path and whether it creates the file, replaces all of it or a part,
      never its content; the working and scratch folders and the files
      written this turn. The paths, the model's words, come as one short
      line each. Never the outputs or the files the model read, where text
      that steers a model comes from, nor the model's explanation, which
      shows to the person: a steered model writes that too.
    - **It's asked about harms, not purposes** (#36): no only for deleting
      or overwriting what wasn't asked for, sending data off the machine,
      changing what runs later, code it can't read, or work unrelated to
      the requests. A prompt by purposes ("stays inside the project") said
      no to 6 of 28 harmless steps on the person's Ollama; this one was
      right on 33 of 33, steering in the command included.
    - **What it lets through shows,** the `$` line or the diff, and a faint
      "auto ran it" or "auto wrote it" with its reason. A command runs as
      anything else no one said yes to (`gitconfig::UNASKED`, the PATH's
      absolute folders).
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
    - **Lokal reads the settings of every folder git runs in**
      (`allowlist::git_folders`): the working folder, and any a `cd`,
      `pushd` or `-C` with its folder written out reaches, from every place
      the line may be, since a `cd` that fails stays where it was. A `cd` is
      taken as bash takes it, logically: `L/..` is where the line was, not
      the folder above where the link `L` leads, and the line starts from
      `PWD` when that's the working folder reached through a link. A folder
      the line has yet to make counts as the repository around it.
    - **git where Lokal can't tell is asked the same way:** after a `cd` it
      can't follow (`cd -`, `cd -P`, `set -P`, `popd`, a variable, a name
      `CDPATH` or `cdable_vars` may send elsewhere, a `cd` behind
      `builtin`, `command` or `eval`, a link the line makes and goes
      through), behind a program that runs another (`env git`, `timeout 60
      git`, `bash -c 'git …'`, `find -execdir git`), or with an option that
      brings settings of its own (`-c`, `--git-dir`, `--work-tree`). It may
      reach a repository Lokal didn't read, or settings of the command's
      own. git's own variables (`GIT_DIR=`, `GIT_CONFIG_COUNT=`,
      `GIT_EXTERNAL_DIFF=`) are the hard core.
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
- **Approve on a guess where a slip costs much.** A question opens on No
  only for the hard core, by Lokal's own rule (`allowlist::opens_on_yes`,
  `src/allowlist/hard.rs`, `files::asking`), never the model's
  judgement. Everything else opens on Yes, so Enter alone approves, and in
  the modes the person picks on Shift+Tab (above) it may need no Enter at
  all. Esc is always No, and a key or a number only moves to its option,
  for Enter to pick, so a stray key never approves. Saying no ends the
  turn, so the person says what to do instead.
  - **The person asked for this (#36):** almost every question used to open
    on No, a slip's cost put on what costs nothing, and a question that
    always opens on No stops being read.
  - **On No, the hard core:** what can't be undone, what leaves the machine,
    and what changes what runs later. It's Lokal's best effort at reading
    bash, never complete: a second session went looking for ways around it
    twice, and what it found is closed (#36), but auto's judge is what
    stands behind it, and #2's box is what will bound it.
    - Root and other machines (sudo, ssh, `wsl.exe`, Windows' script
      hosts), and sends: curl or wget that send (`--variable` included),
      scp, rsync to a remote, nc, `openssl s_client`, `/dev/tcp`, gh's
      writes and `gh auth`, publishing a package, `docker push`,
      `git send-email`, git push with force or delete or to an address or
      a path, and a new remote, by `git remote` or by `git config
      remote.*.url`, `.push`, `.mirror` or `url.*.insteadOf`.
    - Something downloaded and run: piped into a shell or an interpreter
      that reads its code there (`| sh -x`, `| env bash`, `| bash -s
      stable`, `| busybox sh`, `| xargs bash -c`, `| bash -c "$(cat)"`,
      but not `| python3 -c`, where it's data), or handed over to one
      (`bash <(curl …)`, `sh -c "$(command curl …)"`, `eval "$(curl …)"`);
      and `git clone` with `-c` or `--template`, by git or after gh's `--`,
      whose hooks run on the clone. A download that only fills a variable
      (`v=$(curl …)`) is data.
    - Disks and power (mkfs, dd, shred, shutdown), and crontab.
    - A delete or a move (rm, mv, truncate, a `>` that empties a file) of
      anything Lokal can't place inside the working or scratch folder,
      `find -delete` and `rsync --delete`. The folder itself is never
      inside, `.*` may be `..`, a glob with `..` after it climbs out, and
      `mv -t/dir` is read with its folder.
    - An overwrite of a file that exists outside the folders: `cp`,
      `install`, `ln -f`, `tee`, `sed -i`, `curl -o`, `wget -O`, `sort
      -o` (their options read in a group too, as `-sSLo file`), rsync, and
      for cp and rsync each name a glob or a folder brings, as it matches
      now; tar or unzip unpacking outside them; and find running anything
      but a read-only program outside them (`find -L ~`). A path made of a
      variable isn't checked as an overwrite: the judge reads it.
    - git that throws work away (`reset --hard`, `clean`, `restore`, a
      `checkout` of paths, `switch -f`, `stash drop`, `branch -D`,
      `checkout-index -f`, `read-tree -u`), names a program for later git
      commands (`git config core.pager`, `alias.*`: `gitconfig::is_plain`),
      or brings settings of its own (`-c`, and `GIT_*=` but names, dates
      and a pager or editor that runs nothing of the project's, as
      `GIT_PAGER=cat`).
    - docker that reaches the machine or deletes (`-v`, `--privileged`,
      `rm`, `prune`), and a Windows shell's delete, write, download-and-run
      or upload (`ri`, `del`, `robocopy`, `Set-Content`, `iex`, `-InFile`).
    - A line that names where programs run later, keys, or Lokal's own
      files, as whole parts of a path in any of its words (an assignment
      included, a commit's message not), however it uses them and however
      it's spelled (any case, quotes or a backslash inside, a glob that
      could be one of the names): shell start-up, `~/.ssh`, `.git/hooks`,
      autostart, systemd, `.wslconfig`, a PATH folder under home. A `cd`
      into a protected place, an output or a write into one (`cp`, `mv`,
      `ln`, `touch`, `chmod` included), or into what a variable or a glob
      could make one: anything right in a home folder (`~/${f}rc`), or a
      glob that could be a protected name (`AGENT?.md`, `App*`). With no
      place known, a relative path is judged by its own names.
    - A glob where a file named like an option could write (`rg x *`,
      `sed -n 1p *`, whose `-e1e cmd` runs one).
    - A file in a protected place (`protected::is_protected`), in any
      case: what programs run or read as they start, the box's own list
      (`sandbox::PROTECTED`: `.git`, shell start-up, `.vscode`, `.npmrc`…),
      the project's notes (LOKAL.md, AGENTS.md), CI (`.github/workflows`,
      `.github/actions`, `.gitlab-ci.yml`), `.cargo/config.toml`,
      `mise.toml`, `.ssh` and `.gnupg`, a bare repository's hooks, the
      person's settings hidden right in a home folder (`~/.config/…`, or a
      Windows profile's `.wslconfig`), `~/bin`, Windows' `AppData` (its
      Startup folder too) and PowerShell profiles, and a PATH folder. A
      project kept in one (`~/.dotfiles`) is worked on with its commands
      on Yes, and its writes on No. And `remember`, which changes what every
      conversation starts with.
  - **Accepted, and guarded by the judge only:** data written into a GET's
    URL, headers or user agent (`curl "…?q=$(cat .env)"`), until #2's box
    takes the network away; `python3 -m http.server`, which serves the
    folder to the network, since previewing a site is everyday; `mv *
    dir/` with a planted file named `-t..`, which moves files out of the
    folder, where they can be moved back; and a download saved, then run
    in another program of the line (`curl -o i.sh … && bash i.sh`).
  - **On Yes, though Lokal can't see what runs:** a shell or a script,
    xargs, awk, sed, npx, `python -c`, a Windows program, `git push`,
    `commit --amend`, kill, `docker stop`, and a delete or a move inside
    the working folder; a file anywhere that isn't protected, `.gitignore`
    and `.env` included. Enter out of habit runs them; in auto, the judge
    reads them first.
  - **A line Lokal can't read whole** (a variable, a loop, `$(…)`, a
    here-document) is read roughly (`hard::rough_programs`), by a small
    reader of bash: quotes and backslashes as bash takes them (`r''m` is
    rm), a program at every `;`, `|`, `&`, line break, `(` and group
    brace, braces expanded as bash would (`.g{i,}t` is `.git`), and a
    substitution read as programs of its own while the word it's in goes
    on after it (`curl "…$(date)" -o out.json` keeps its `-o`). Each
    program is taken as run from every place a `cd` in the line reaches,
    and the line is the hard core if any is.
    - **A program named by a variable or a substitution** (`$X -rf`,
      `$(echo rm) -rf`, `$'\x72m'`) is the hard core, and so is `$'…'`
      handed to a shell. Elsewhere it's a word (`cut -d$'\t'`).
    - **A here-document's text is data** unless a shell, an interpreter,
      `eval` or `source` reads it: a commit message or a README's lines are
      no programs.
    - **A path made of a variable is never placed,** and after a `cd` Lokal
      can't follow, no place is.
  - **Shell words that lead into a program** (`if`, `then`, `do`, `!`,
    `time -p`, `coproc`, `function f`) are skipped in either reading, so
    `if …; then sudo …; fi` is sudo; a loop's list holds no program.
  - **The rule only picks where the question starts,** never whether there is
    one, except in accept edits and auto, which the person picked (above).
  - **A working folder that holds every file doesn't count**
    (`tools::holds_everything`): the home folder, /, and under WSL a
    Windows drive, its Users folder or a profile in it, where WSL often
    starts when opened from Windows. There a delete or a move opens on
    No, and a file change isn't accepted unasked: in auto, the judge
    decides it.
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
  - **Nor a key meant for a message being typed** (#3): what's typed while
    Lokal works goes to a draft in Lokal's memory (`queue.rs`), never the
    terminal's buffer, so it can't answer a question. While there's a
    draft, or a key went into one in the last second, a question takes only
    Esc and Ctrl+C for its first 600 ms (`menu::choose`'s `deaf`): an Enter
    meant to queue the draft would approve a step the person hasn't read.
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
    by their code (`<U+202E>`), as are the characters that show as nothing
    (zero-width ones, the Tags block), and sending as a step.
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
- **Let an MCP server act without a yes, or give the model its tools
  before the person has seen them whole** (`src/mcp.rs`, #37). A server is
  a program the person named, and its tool descriptions are a third party's
  text that goes along with every request. What a call gives back is an
  output, shown and cut as a command's is.
  - **Servers are set in the person's settings only** (`mcp.<name> =
    <command line>`, optionally `mcp.<name>.tools = a, b`), never a
    project's `.mcp.json`, or a cloned repository could start programs.
    The settings are Lokal's own, so the model can't add one.
  - **A server starts in the home folder, never the project's:** a
    project's `.npmrc` or `uv.toml` could change what `npx` or `uvx`
    fetches and runs. Its program comes from the PATH's absolute folders,
    as for what runs unasked, and it runs in a session of its own, ended
    with Lokal (stdin closed, then SIGTERM, then SIGKILL), a closed
    terminal or a SIGTERM to Lokal included (`client::end_with_lokal`).
    A SIGKILL to Lokal leaves them running, since nothing catches it;
    most end anyway when their stdin closes.
  - **A server can't hang a turn or fill Lokal's memory:** its stdin is
    written on a thread of its own, so Esc still stops a call to one that
    stopped reading; only the answer awaited is passed on; a line is at
    most 32 MB and an image 20 MB, and its stderr is read in bounded
    pieces.
  - **A tool list shows before the model gets it:** each tool's name,
    description and parameters as the JSON the model gets, control and
    hidden characters as marks, asked on No ("Offer them to the model?").
    A curated view would leave a place for words to hide. A long one shows
    its first lines and how many are left out, the rest a choice away, as
    a long diff does. Its hash is kept in the state
    folder (`mcp-seen`), and a list that changes is shown again. The list
    is fixed for the session, so the prompt cache holds.
  - **Every call asks, on No** ("Call it?"), with the server, the tool and
    its arguments as the JSON the server gets: a line break inside a value
    shows as `\n`, so no value passes for another parameter. Long ones
    show their start and count the rest. "Don't ask again" covers that exact call for the
    session, kept apart from commands, so a call's text can't let a
    command run unasked. A tool's name alone would be `cmd *`. Auto never
    judges a call, mail never approves one, and `readOnlyHint` is the
    server's word, never Lokal's rule. Arguments that aren't an object
    are turned down before the question, which couldn't show them.
  - **What a call gives is an output:** text cut to the room left, an
    image to a model that sees, anything else a line saying so. A server's
    words on screen, its last stderr lines included, show their control
    characters as marks.
  - **Stdio and tools only:** no HTTP (TLS and OAuth, and an address Lokal
    would talk to), no sampling (a server writing to the person's model),
    no resources or prompts, and a server's `instructions` aren't passed
    on. Both eras: `initialize` first, then `server/discover` with `_meta`
    (2026-07-28) when that fails.
  - **Conversations at a terminal only:** a one-shot request, or a
    conversation without a terminal, starts no server, since nothing
    there could be asked about.
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
- **Grow on disk.** It keeps the last conversation only, a line an MCP
  server for the tool list the person saw, and user.md is the person's own
  file.
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
  - **Short first (#45):** one line, the fact first, and a why only where
    it changes what the person does next ("Reasoning on for this
    session."). An approval question, the auto warning, the reason a
    question opens on No, and an error that says what to do keep theirs:
    cut short there, Lokal reads as offhand where it needs to be taken
    seriously, and a No without its reason looks like a bug. What goes to
    the model (a bracketed note in an output) is written for the model,
    not shortened for the screen.
  - **The page's terminal plays the real screen** (`site/index.html`): the
    approval's options, the working labels, the prompt's status, the
    spinner's frames, the reasoning window (as `/reasoning` shows it), the
    times beside the messages and the two pulses, at Lokal's timings. When one of them changes here, it
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
    - **The words win the room:** a row without two spaces to spare goes
      without, and a slash command gets none.
    - **On screen only:** the model, the saved conversation and the other
      sessions get nothing of it, and a one-shot request shows none.
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
| `compact.rs` | `/compact`: what's summed up, the request that asks for it, the pair that takes its place |
| `progress.rs` | What the spinner says while the model works: time, tokens, speed, the file being written |
| `chat.rs` | One streamed chat request on its own thread |
| `answer.rs` | What of the stream is shown: no reasoning, no blank edges |
| `markdown.rs` | Bold, inline code, headings, code blocks and links, styled as the answer streams |
| `server.rs`, `http.rs` | Finding the server and its model; plain-HTTP requests |
| `tools.rs`, `shell.rs`, `diff.rs` | The model's tools, running a command, showing a change |
| `tools/command.rs` | run_command and the person's `!` commands: who decides, what the model hears |
| `tools/files.rs` | read_file, write_file and edit_file, and how a change asks: protected places, accepted edits, auto's judge |
| `image.rs` | What an image is to the model: its kind and size from its first bytes, its data URL, what it takes of the context |
| `tools/remember.rs` | remember: a line added to user.md, or one swapped for it |
| `tools/mcp.rs` | A call to an MCP server's tool: shown, asked, and what it gave as an output |
| `mcp.rs` | The session's MCP servers: started, their tool lists shown to the person, offered, ended |
| `mcp/client.rs` | One MCP server's process and the JSON-RPC with it, in both eras of the protocol |
| `mcp/seen.rs` | The tool lists the person has seen, a hash a server |
| `tools/mail.rs` | list_sessions and mail, shown as steps |
| `allowlist.rs` | Which commands run without asking, and which questions open on Yes |
| `allowlist/hard.rs` | The hard core: what a command's question opens on No for, and what auto's judge never decides |
| `protected.rs` | Where a change opens on No in every mode: what programs run or read as they start, keys, the person's settings |
| `gitconfig.rs` | What a repository's own git settings and hooks would have git run |
| `keys.rs` | The keyboard while Lokal works: Ctrl+C and Esc stop it, other keys go to the draft |
| `queue.rs` | What's typed while Lokal works: the draft under the spinner or the answer, the lines Enter queued, where they go when the turn ends |
| `input.rs` | The `>` prompt (hints, completion, Ctrl+C twice, an image pasted as `[Image #N]`) and approvals |
| `clipboard.rs` | The clipboard's image, read on the person's key by Lokal's own fixed programs |
| `clock.rs` | When a message was sent, as the screen shows it: the local time, and the date on a new day |
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
| `.github/actions/publish/` | guimadaleno/base/release's publish action, **copied unchanged**: signs the manifest and uploads the release |
| `.github/workflows/ci.yml` | Formatting, lints and tests on every push to main and every pull request |
| `tools/icon.py` | Draws the mark and writes `site/`'s icons from one definition |

## Invariants

- **`src/release.rs` and `tests/release/`, `src/report.rs` and
  `tests/report/` are copies, changed only at their source:**
  `guimadaleno/base/release` (`client/release.rs`, `testdata/`) and
  `guimadaleno/base/issues` (`client/report.rs`, `testdata/`). That's why both
  modules are declared with `#[rustfmt::skip]` and `#[allow(clippy::all)]`:
  an edit here makes the copies disagree.
  - **So is `.github/actions/publish/`,** `guimadaleno/base/release`'s
    `publish/` as of base's 2541007, `release-v1.2.0`'s code with examples
    that name no app: base is private, and a public repository can't use an
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

Lokal finds Ollama on its own on port 11434, after LM Studio's 1234 at
each place it looks (`server::candidates`), or takes it from `server =
localhost:11434`, and reads what it says about the model. Without `model =`,
it takes the first loaded model that uses tools, or else the first listed
that does (`server::pick_ollama_model`): Ollama lists every model pulled, an
embedding model too. Last checked against **Ollama 0.40.15**, on
2026-11-09.

| Lokal relies on | Where | How to check |
|---|---|---|
| `GET /api/version` answers `{"version": "..."}`, which is how Lokal knows it's Ollama | `server.rs` (`is_ollama`) | `curl <host>:11434/api/version` |
| `POST /api/show` with `{"model": "<id>"}` gives `capabilities` (`vision` when the model sees, `tools` when it calls them), `model_info["<general.architecture>.context_length"]` (the most it takes) and `parameters`, a text with the model's own `num_ctx` when it has one | `server.rs` (`ollama_model`) | `curl <host>:11434/api/show -d '{"model": "<id>"}'` |
| `GET /api/ps` lists the loaded models, each `name` tagged (`:latest` when none was given), with `context_length` | `server.rs` (`loaded_context`) | `curl <host>:11434/api/ps` |
| `max_tokens` caps a reply on `/v1/chat/completions` | `conversation.rs` | a request with `"max_tokens": 40` and a long question |
| Reasoning streams in `delta.reasoning`, beside an empty `content`, never `reasoning_content` (0.40.2, 2026-11-12) | `chat.rs` (`reasoning_in`) | a streamed request to a model whose `capabilities` list `thinking` |
| A tool call comes whole once the model has written it, with nothing streaming meanwhile: 19 s for a file of 60 lines, after a sentence of text (0.40.2). So text that stops for 1.5 s is ended there, with its blank line, and the spinner shows under it, the time alone while nothing comes (`TEXT_PAUSE`) | `conversation.rs` (`ask_model`) | a streamed request with one tool that asks for a sentence, then a file written with the tool |
| An `image_url` part with a base64 `data:` URL in a `tool` message's content reaches a model with `vision` (tried with Qwen 3.8 27B and LFM3-VL 2B); a GIF gets HTTP 400 (`invalid image input`). An image adds about pixels / 1024 tokens with Qwen and with LFM3-VL, up to about 4 MP, where Ollama shrinks it | `image.rs`, `files.rs` | the same request as LM Studio's, with a PNG, a JPEG, a WebP and a GIF |

**The page's "Set up a model" tabs name each server's screens and
options:** LM Studio's Developer tab, "Serve on Local Network" and the mark
on models that use tools; Ollama's context slider, `OLLAMA_CONTEXT_LENGTH`,
`OLLAMA_HOST` and the `tools` that `ollama show` lists (from its FAQ and
context-length pages, 2026-11-11). When either changes, check those words
in `site/index.html` and `README.md` too. Both also list what Lokal won't do: a change to the
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
  `guimadaleno/base/release`'s action, copied into `.github/actions/publish/`.
  Read its README before
  touching `update.rs`, `install.rs`, the workflow or the key.
- **Every tag reaches every installed copy, unasked,** so a tag is pushed
  only when the user says so.
- **`Cargo.toml`'s version, the tag and a `## <version> — <date>` entry in
  `CHANGELOG.md` must agree;** the workflow refuses a tag otherwise. The
  entry is the release's notes, written for the person using Lokal.
- **Bump the smallest part that fits:** patch for fixes, minor for anything
  new, while below 1.0.

## Keys and secrets

- **`update.pub` is the public half of Lokal's release key,** compiled in.
  The private half lives in the repository's `UPDATE_KEY` secret and in the
  user's offline backup, never in this repository. Losing it leaves every
  installed copy without updates.
- **`R2_ACCESS_KEY_ID` and `R2_SECRET_ACCESS_KEY` are the repository's
  secrets** for the `guimadaleno-update-lokal` bucket, and are scoped to that
  bucket only.
- **The R2 endpoint in `.github/actions/publish/action.yml` names the
  Cloudflare account's ID.** It's no secret, the upload can't do without
  it, and the token above is what guards the bucket.
- **The page publishes with the same R2 token,** like the other apps' pages:
  `site.yml` copies `site/` into `page/` in Lokal's bucket, never deleting,
  and the site's Worker serves that prefix at guimadaleno.com/lokal/. Without
  the token, `site.yml` does nothing.

## Measured

| What | Measured |
|---|---|
| Release binary (static, stripped) | 2.9 MB (0.10.0) |
| Start to the prompt, server found | ~40 ms (0.8.1) |
| Memory at the prompt | 2.4 MB (0.8.1) |
| Auto's judge, a 27B model with reasoning off (Ollama 0.40.2) | 0.3–2 s a step; 6–22 s with reasoning; the conversation's prompt cache stays |

Measured on a current laptop, under WSL; the judge on a consumer GPU. The
page's "Light" column and the README quote the first three: a change that
moves them past what's written there changes the words too.

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
  - **And the draft (#3):** typing, Backspace, Ctrl+U and Ctrl+W, Enter,
    a bracketed paste with line breaks, Esc and Ctrl+C with a draft, and an
    approval appearing mid-typing, all tried in a pseudo-terminal against a
    fake server. Still owed: the same in Windows Terminal, whose paste and
    keys may arrive differently.
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
- **Images on LM Studio with a model family other than Qwen:** the image
  goes in the read's `tool` message. On Ollama 0.40.15, Qwen 3.8 and
  LFM3-VL both see it there; on LM Studio, only Qwen was tried, and LM
  Studio renders each model's own chat template, which could drop an
  image in a tool result and leave the model answering blind. A user
  message after the result is the fallback (#17).
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
- **Ollama on Windows, found from WSL:** Lokal looks at the Windows host's
  11434 as it does at 1234, and the page says to set `OLLAMA_HOST` to
  `0.0.0.0:11434` there. Only Ollama inside WSL has been found on its own
  (0.40.2, 2026-11-11). Still owed: Ollama on Windows found from WSL.
- **Servers other than LM Studio:** the chat path should work with any
  OpenAI-compatible server through `server = host:port`. Only Ollama 0.40.2
  has been tried, under WSL: a conversation with a command step, in auto.
  The context and its gauge come from LM Studio's `/api/v0/models`, or
  from Ollama's model `num_ctx` or `/api/ps` (above); elsewhere they stay
  unknown. On Ollama, without the model's own `num_ctx`, Lokal reads
  `/api/ps` again after the session's first reply: the model may not have
  been loaded at the start, or another client may have loaded it with
  options of its own, and Lokal's request loads it as its requests find
  it. The notes are cut to the new room at once, the system prompt
  follows between turns, and the warning about a small context shows in
  a conversation only. A reload by another client's options is read from
  Ollama's scheduler, not seen.
  The speed is timed by Lokal from the first token on. The spinner counts
  a token a piece, and so does the prompt's status where a server sends no
  `usage.completion_tokens`: tried with a fake server that sends none.
- **MCP servers out in the world:** tried against `tests/mcp/fake_server.py`
  in both eras, misbehaving servers (hidden text, one that stops reading,
  one that outlives its stdin), and one real server, Vemka's
  (`vemka.exe --mcp`, a Windows program found through WSL's PATH), with
  Qwen 3.8 on Ollama calling it, seeing a screen and leaving. A published
  server through `npx` was refused by a permission check, and a real one
  that speaks only 2026-07-28 hasn't been met yet.
- **Hooks in auto, the unasked PATH and control characters as marks:** run
  in a pseudo-terminal against a fake server only. A post-commit hook made
  auto ask on No, and without it the judge let the commit through; an
  answer and an output holding ESC and OSC 52 showed `^[` marks. Still
  owed: the same with a real model, and the PATH with a `.` in it.
- **The hard core and auto's judge on file changes** (#36):
  - **Against a scripted server, in a pseudo-terminal, with Lokal started
    in `~`:** in manual, a file on the desktop opened on Yes, `sed -n` ran
    unasked, a loop opened on Yes, and `rm -rf ~/Documents` and `~/.bashrc`
    opened on No. In auto, the file and the loop went through the judge.
  - **With Qwen 3.8 27B on Ollama 0.40.2, in the person's setup:** a site
    written under `/mnt/c/Users/…/Desktop` in auto with no question, its
    `wc -l` run unasked, and its `rm -rf` asked on No.
  - **Still owed:** the same on LM Studio, and the rough reading of lines
    Lokal can't read whole against what models write over a long session.

## Later, not decided

Tracked as issues:
- **other local servers** (#1, Ollama first);
- **auto inside a sandbox**, so the judge's mistakes stay in a box the
  kernel enforces (#2). Its design is in `docs/sandbox.md`: read it before
  touching how commands run unasked;
- **the live `>` while Lokal works** (#3), with the answer printed above
  it, only if the draft under the spinner turns out not to be enough;
- **`/compact` on its own when dropping outputs isn't enough** (#4); the
  command is in;
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


Decided, and kept here so they aren't undone:

- **The MIT license** (`LICENSE`), chosen as the repository went public.

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
Guilherme Madaleno, and https://guimadaleno.com. **The repository,
https://github.com/guimadaleno/lokal, is public:** `Cargo.toml` names it, and
the page links to it under the install line and in the footer.
