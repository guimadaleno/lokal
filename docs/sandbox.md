# The box (#2)

A design, not built yet. Commands that run without the person's yes run in a box the kernel enforces. Then a hole in reading a command's text (a glob, a brace) or a judge's mistake stays inside the box.

## What the box promises

A command inside it:

- **Writes** only in the session's scratch folder and a private `/tmp`, plus the project folder in the work box (below), whose `.git` stays read-only.
- **Has no network:** no TCP, no UDP, no DNS.
- **Reaches no socket** outside it: `docker.sock` (root on the machine), the ssh and gpg agents, dbus, and other Lokal or Claude sessions.
- **Runs nothing of Windows':** no `.exe`, nothing on `/mnt/<drive>`, no `wslpath`, which is `/init`.
- **Signals no process outside it,** Lokal included.
- **Doesn't see the person's keys:** `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/gh`, `~/.kube`, `~/.docker`, `~/.netrc` and `~/.git-credentials`, nor Lokal's own files. They're covered by empty mounts.

One hole voids the promise, so each line has a test that tries to get out (below).

## How

Lokal runs itself again as `lokal --box <profile> -- bash -c <command>`. A fresh process with one thread can set up mounts safely, which the forked child of a threaded Lokal can't. That process:

1. **Unshares** the user, mount and network namespaces, and maps the person's uid to itself. Only `lo` exists in the new network namespace.
2. **Mounts:**
   - a tmpfs over `/run` and `/var/run`;
   - a fresh tmpfs at `/tmp`, with the scratch folder bound back into it;
   - empty folders over the secrets above;
   - in the work box, read-only binds over what would have something run or
    steer the model later: `.git`, `.vscode`, `.idea`, `.envrc`, shell rc
    files, `.gitconfig`, `LOKAL.md` and `AGENTS.md`;
   - an empty `binfmt_misc` of its own, so WSL's interop doesn't apply inside.
3. **Applies Landlock**, then `no_new_privs`:
   - writes are allowed beneath the writable folders above only;
   - execution is allowed beneath `/usr`, `/bin`, `/sbin`, `/lib*`, `/opt`, the home folder and the project, never `/init` or `/mnt`;
   - the signal and abstract-socket scopes are on.
   - It needs Landlock ABI 6 (Linux 6.12), the first with the scopes.
4. **Filters two calls with seccomp:**
   - `socket(AF_UNIX)` is refused, since Landlock doesn't cover connecting to a socket by its path and a socket could sit anywhere; `socketpair` still works;
   - `io_uring_setup` is refused, since io_uring could open a socket past the filter.
   - Calls of the 32-bit and x32 ABIs are refused whole.
5. **Drops variables** that point outside: `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `WSL_INTEROP`, `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `DISPLAY` and `WAYLAND_DISPLAY`.
6. **Execs bash.** It runs as the person's uid, with no capabilities left.

Everything is syscalls through `libc`: no bubblewrap and no new crate. Setting it up costs about 2–3 ms a command (measured with `unshare`).

**Checked on WSL with kernel 6.18 and Landlock ABI 7, by hand with `unshare` and a Landlock probe:**

- Writes outside the allowed folder are denied.
- `kill` on the parent is denied.
- `wslpath` is denied, as is a `.exe` on `/mnt/c` or one copied into an allowed folder, since interop's interpreter is `/init`.
- An empty `binfmt_misc` in the user namespace alone stops `cmd.exe`.
- The network namespace holds only `lo`.
- A read-only bind gives "Read-only file system".

## Two boxes

- **The read box**, for the read-only list (`allowlist.rs`). Only the scratch folder and `/tmp` are writable.
  - Whatever slips past reading the text can't write or reach out.
  - `docker` read-only commands (they need the socket) and `wslpath` stay outside the box, as today.
- **The work box**, for what auto's judge lets through. It also makes the project writable.
  - Only in a git repository, and never with a working folder of `~` or `/`.
  - Writes go straight in, except to the read-only files above. After the command, Lokal lists the files it changed, from `git status`. That was chosen over an overlay to review first, which would be slower to apply for builds.

**What the person approves runs outside the box, as today.** It may need the network (`git push`, `npm install`).

**When a command hits the box** (EACCES, EROFS, ENETUNREACH), its output goes to the model with a note: it ran in Lokal's box, with no network and writes only in the named folders. To run it outside, the model proposes the command again, and the person is asked.

**When the box can't be set up** (an older kernel, or Ubuntu's AppArmor refusing unprivileged user namespaces), it fails closed:
- read-only commands run as today;
- auto lets nothing through, and every command asks, with a note once a session saying why.

## What stays open

- **A command in the work box changes project files without a diff.** It could plant a line in a `Makefile` that runs when the person later approves `make` outside the box. The read-only files stop the quiet routes (hooks, editor tasks, the project's notes for the model), and the list of changed files shows the rest. The box bounds the machine, not the project.
- **Nested namespaces:** a command inside can make its own user namespace. Mounts it inherits are locked there, and Landlock forbids changing the mount topology, so it can't uncover what's hidden. The escape tests check it.
- **Reading isn't hidden** beyond the masked secrets. With no network, nothing leaves from inside, but what's read reaches the model.
- **A kernel bug** in namespaces or Landlock would void the box.

## Escape tests

One test per promise, run from inside the box:

- writing to `~`, the real `/tmp`, `/mnt/c`, `.git/hooks`, and through a symlink or a hardlink that points out;
- TCP and UDP to an address, and DNS;
- connecting to `docker.sock`, the dbus socket, a Lokal session's socket and an abstract socket;
- running a `.exe` from `/mnt/c` and a copy in the project, and `wslpath`;
- `kill` and `ptrace` on Lokal;
- `unshare`, `mount` and `umount` to undo a layer;
- reading `~/.ssh`.

Where the box can't be set up, the tests say so and pass.

**A working folder under /tmp comes along:** it is cloned before /tmp is covered and put back, read-only in the read box. The scratch folder goes back over it, writable. So does `/tmp` itself, the person's other files there unchanged.

## Phases

1. `sandbox.rs`, `lokal --box` and the escape tests (`tests/sandbox.rs`), with no change in behavior. **Done:** the 13 tests pass here, and taking out any one layer (the hidden `/run`, seccomp, the scopes, the hidden keys, the read-only folder under /tmp) makes one fail. About 7 ms a command, bash included, with the release build.
2. The read-only list runs in the read box.
3. Auto runs what its judge lets through in the work box, and lists the files changed.
4. After real use, decide whether the box replaces the judge, which was this issue's first idea.
5. Maybe: network to chosen domains through a proxy, as Claude Code's sandbox does. Its own docs warn that domain fronting gets past such a list, so the box stays with no network until there's a real need.

Claude Code's sandbox (bubblewrap, a network proxy with allowed domains, protected files in the working folder) was read for comparison. The protected files came from it.
