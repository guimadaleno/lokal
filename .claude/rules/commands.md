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

# Commands

How commands will run unasked in a box is designed in `docs/sandbox.md` (#2): read it before touching that.

## What runs without a yes

- **A `!` line at the `>` is the person's own command:** typing it was their yes (`tools::run_own`). It runs as the model's do, but with the person's PATH and without `gitconfig::UNASKED`, which guard only what runs unasked. Its output goes with their next request, never with a turn mail starts.
- **Commands run in a session of their own** (`setsid`): nothing can wait on the terminal (sudo fails at once instead of hanging), and Ctrl+C reaches a command only through Lokal.
- **What runs unasked finds programs only in the PATH's absolute folders** (`shell::run`): with `.` or an empty entry, `ls` could be a program a project planted.
- **git runs programs a repository's own settings name, even to read** (fsmonitor, filters, diff drivers, `gpg.program`), from files in `.git` anything could have written. So git runs unasked only when the repository's settings, not the person's global ones, name none (`src/gitconfig.rs`); otherwise it asks, on No, with the setting named, whatever was approved before.
  - Lokal reads the settings of every folder git may run in (`allowlist::git_folders`): the working folder and any a written-out `cd`, `pushd` or `-C` reaches, from every place the line may be, a `cd` taken logically, as bash does. A folder the line has yet to make counts as the repository around it.
  - Where it can't tell, git asks the same way: after a `cd` it can't follow, behind a program that runs another (`env git`, `bash -c`, `find -execdir`), or with an option that brings settings (`-c`, `--git-dir`, `--work-tree`). git's own variables are the hard core.
  - Unasked, git gets fsmonitor off and stays out of submodules (`gitconfig::UNASKED`), whose settings Lokal doesn't read; a git option about submodules is asked about. A git too old to say where a setting comes from asks for every command.
- **A working folder that holds every file doesn't count** (`tools::holds_everything`): home, `/`, and under WSL a Windows drive, its Users folder or a profile, where WSL often starts. There a delete or a move opens on No, and a file change isn't accepted unasked.

## Auto

- **The one place a model decides** (`src/judge.rs`): asked apart from the conversation, whether a command that would open on Yes runs without the question, and whether a file change outside the working and scratch folders is written (rule `files`).
- **The person is told, once a session, that it will get some wrong and is used at their own risk:** as they pick it (`mode::show_warning`) or as a session starts on it from the settings (`mode::show_start_warning`); never over a line being typed, and at the latest before the first step it decides.
- **It can only take away:** its yes runs the step; its no, or a reply Lokal can't read, asks the person, on No, with its reason. A judge wrong toward caution costs an Enter.
- **It never decides Lokal's floor:** the hard core; git with repository settings that run a program, or after a `cd` Lokal can't follow; a git command when the repository has an active hook or any setting beyond the plain ones git writes itself (`gitconfig::write_program`: what's let through is the short list that can't run anything, since what can has no end; the person's global settings and hooks don't count). Read-only commands and "don't ask again" approvals run as in every mode.
- **It reads only the person's or Lokal's own words:** the person's last three requests as typed, the command, or a file change's path and kind (create, replace, part), never its content; the working and scratch folders and the files written this turn; paths as one short line each. Never the outputs, the files read or the model's explanation, where steering comes from.
- **It's asked about harms, not purposes** (#36): no only for deleting or overwriting what wasn't asked for, sending data off the machine, changing what runs later, code it can't read, or work unrelated to the requests. A prompt by purposes refused harmless steps.
- **What it lets through shows** (the `$` line or the diff, with a faint "auto ran it" or "auto wrote it" and its reason) and runs as anything unasked does.
- **The model's own code still runs as the person:** a script it wrote and then ran is a command the judge sees, not what the script does. Only the box (#2) bounds that.

## The hard core: what opens on No

- **The person asked for this (#36):** a question that always opens on No stops being read. The rule only picks where the question starts, never whether there is one, except in accept edits and auto, which the person picked.
- **Best effort at reading bash, never complete:** auto's judge stands behind it, and the box (#2) will bound it. What a session sent to find ways around it found is closed (#36).
- **On No** (`allowlist::opens_on_yes`, `src/allowlist/hard.rs`):
  - Lokal itself (#61), by its name or a `/proc/*/exe`, behind a wrapper, a shell or a terminal of its own (tmux, screen, expect) too: `lokal mcp add` writes its settings, and a scripted terminal could answer its question. Behind another program it counts when followed by `mcp`, an option or nothing, so a folder named lokal isn't it. A copy under another name, or `cargo run` in Lokal's own repository, isn't caught: the judge reads it.
  - Root and other machines (sudo, ssh, `wsl.exe`, Windows' script hosts), and sends: curl or wget that send, scp, rsync to a remote, nc, `/dev/tcp`, gh's writes, publishing, `docker push`, git push with force or delete or to an address, a new remote.
  - Something downloaded and run: piped into a shell or an interpreter that reads its code there (not `| python3 -c`, where it's data), or handed to one (`bash <(curl …)`, `eval "$(curl …)"`); `git clone` with `-c` or `--template`. A download that only fills a variable is data.
  - Disks, power and crontab.
  - A delete or a move of anything Lokal can't place inside the working or scratch folder; the folder itself is never inside.
  - An overwrite of a file that exists outside those folders, unpacking outside them, find running anything but a read-only program outside them. A path made of a variable isn't checked as an overwrite: the judge reads it.
  - git that throws work away, names a program for later git commands (`gitconfig::is_plain`), or brings settings of its own (`-c`, `GIT_*=` but names, dates and a pager or editor that runs nothing of the project's).
  - docker that reaches the machine or deletes, and a Windows shell's delete, write, download-and-run or upload.
  - A line that names where programs run later, keys or Lokal's own files, as whole parts of a path in any word, however spelled; a `cd`, an output or a write into one, or into what a variable or a glob could make one. Lokal's own files count by what they are, under any name a link gives them (#57).
  - A glob where a file named like an option could write (`rg x *`).
  - A file in a protected place (`protected::is_protected`). A project kept in one (`~/.dotfiles`) runs its commands on Yes and its writes on No. And `remember`, which changes what every conversation starts with.
- **Accepted, guarded by the judge only:** data in a GET's URL, headers or user agent, until the box takes the network away; `python3 -m http.server`, since previewing a site is everyday; `mv * dir/` with a planted `-t..`, whose files can be moved back; a download saved, then run by another program of the line.
- **On Yes though Lokal can't see what runs:** a shell or a script, xargs, awk, sed, npx, `python -c`, a Windows program, `git push`, `commit --amend`, kill, a delete or a move inside the working folder, and a file anywhere unprotected (`.gitignore` and `.env` too). In auto, the judge reads them first.
- **A line Lokal can't read whole** (a variable, a loop, `$(…)`, a here-document) **is read roughly** (`hard::rough_programs`), as bash would split and expand it, each program from every place a `cd` in the line reaches; the line is the hard core if any program is.
  - A program named by a variable or a substitution is the hard core, and so is `$'…'` handed to a shell.
  - A here-document is data unless a shell, an interpreter, `eval` or `source` reads it.
  - A path made of a variable is never placed, and after a `cd` Lokal can't follow, no place is.

## Don't ask again

- **The exact command, never a pattern:** `docker *` would let in `docker run -v /:/host …`, and a file the model reads could steer it to use one.
- **A project's approvals live in the person's settings** (`~/.config/lokal/approved`), never in the project, or a cloned repository could grant itself commands. The project is the git root around the working directory, or the directory itself.
