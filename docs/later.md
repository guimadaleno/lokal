# Later, not decided

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
