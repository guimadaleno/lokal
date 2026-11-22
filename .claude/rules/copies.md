---
paths:
  - "src/release.rs"
  - "src/report.rs"
  - "src/main.rs"
  - "tests/release/**"
  - "tests/report/**"
  - ".github/actions/publish/**"
  - ".github/workflows/release.yml"
---

# Copies

- **`src/release.rs` with `tests/release/`, and `src/report.rs` with `tests/report/`, are copies, changed only at their source:** guimadaleno/download and guimadaleno/issues (`client/`, `testdata/`). Both modules are declared with `#[rustfmt::skip]` and `#[allow(clippy::all)]`: an edit here makes the copies disagree.
- **So is `.github/actions/publish/`,** guimadaleno/download's `publish/`: that repository is private, and a public one can't use its action (#42). A newer one comes in as a fresh copy, its commit named in `release.yml`'s comment.
