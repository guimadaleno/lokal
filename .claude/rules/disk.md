---
paths:
  - "src/scratch.rs"
  - "src/conversation.rs"
  - "src/install.rs"
  - "src/config.rs"
  - "src/update.rs"
---

## Grow on disk

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
