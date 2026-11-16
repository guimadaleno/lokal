---
paths:
  - "src/allowlist.rs"
  - "src/allowlist/**"
  - "src/judge.rs"
  - "src/mode.rs"
  - "src/gitconfig.rs"
  - "src/shell.rs"
  - "src/tools.rs"
  - "src/tools/command.rs"
  - "src/approvals.rs"
  - "src/sandbox.rs"
  - "src/protected.rs"
  - "src/tools/files.rs"
  - "src/input.rs"
  - "src/conversation.rs"
  - "src/main.rs"
---

How commands run unasked is designed in `docs/sandbox.md` (#2): read it
before touching that.

## Run a command that may change something without a yes

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
  the working and scratch folders be written (`files.md`).
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
  - **A turn mail started stays manual,** as in accept edits (`files.md`).
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

## Approve on a guess where a slip costs much

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
  one, except in accept edits and auto, which the person picked (above, and `files.md`).
- **A working folder that holds every file doesn't count**
  (`tools::holds_everything`): the home folder, /, and under WSL a
  Windows drive, its Users folder or a profile in it, where WSL often
  starts when opened from Windows. There a delete or a move opens on
  No, and a file change isn't accepted unasked: in auto, the judge
  decides it.

## Approve more than the person saw

- **A pattern like `docker *` would let in commands nobody saw**
  (`docker run -v /:/host …`), and a file the model reads could steer it to
  use one.
- **The project's approvals live in the person's own settings**
  (`~/.config/lokal/approved`), never inside the project, or a cloned
  repository could grant itself commands that run unasked.
- **The project is the git root around the working directory,** or the
  directory itself. The file keeps its newest 500 lines.
