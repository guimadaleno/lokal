---
paths:
  - ".github/**"
---

# Stack: GitHub Actions

On-demand module of `CONVENTIONS.md` — read before writing or changing a
workflow. Each stack's own module adds what its tools need (`stack-rust.md`:
what Cargo caches, what its CI runs).

Every repository is private, so every minute comes out of the month's
included compute. A run proves what a push could break, on the cheapest
runner that can prove it, and stops the moment it can't.

- **Every job has `timeout-minutes`,** about three times its slowest normal
  run. Without it a job lives six hours: one `apt-get` stuck on a mirror
  once cost two thirds of a month's spend.
- **A download from outside is cut off and tried again,** not waited on:
  each try under `timeout`, three tries, the step with its own
  `timeout-minutes`. A stalled mirror, not the code, is what has hung jobs.
- **Linux by default.** A Windows minute costs about 1.7 Linux ones and a
  macOS minute about ten, and every job is rounded up to the minute. What
  builds or tests on Linux (a cross-compile, a lint, a core that names no
  system) runs there; Windows and macOS run only what needs that system.
- **A platform the product doesn't ship runs nothing.** Its job waits,
  commented, until its shell comes.
- **A push runs only what it could break.** CI ignores prose by naming its
  files, never `**.md`, which would catch one compiled in; a workflow for one
  part (the site, a release) runs on that part's paths or its tag.
- **Push on main, pull requests for the rest.** Both on every branch run a
  branch twice.
- **A newer push cancels the older run** in CI; never in a release or a
  deploy, which must finish.
- **Dependencies are cached, keyed by the lockfile.** Each stack's module
  names what to cache.
- **Artifacts keep `retention-days`:** a day for what passes between jobs, a
  week for what a person downloads.
- **Runner images are pinned by name, never `-latest`.** The image is part of
  what ships, so moving to a newer one is a commit and a release to look at.
