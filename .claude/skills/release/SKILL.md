---
name: release
description: "Use before changing Lokal's version, tagging or publishing, and before touching update.rs, install.rs, the release workflow or the update key: where Lokal's release departs from gms-desktop-release."
---

# Release

gms-desktop-release applies, but for these.

## Versioning and releases

- **`Cargo.toml`'s version, the `v<version>` tag and a `## <version> — <date>` entry in `CHANGELOG.md` agree,** or `release.yml` refuses the tag. That entry is the release's notes, in English only, written for the person using Lokal.
- **The release build is tried in WSL, never on a virtual machine,** at the owner's word: Lokal is a Linux program. It's built as `release.yml` builds it, put in place with `lokal --install` under a scratch `HOME`, and run from there, never over the owner's own copy.
- **A page change that announces the version waits for the tag's run** (rule `page`).
- **The R2 endpoint in `.github/actions/publish/action.yml` and `site.yml` names the Cloudflare account's ID:** it's no secret, the upload can't do without it, and the bucket's token is what guards the bucket.
