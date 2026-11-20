---
name: release
description: "Use it before changing Lokal's version, tagging or publishing, and before touching update.rs, install.rs, the release workflow or the update key: how a version is tagged, what a release publishes, where the keys and secrets live, and base's release protocol to read first."
---

## Versioning and releases

- **A release is a `v<version>` tag.** `.github/workflows/release.yml`
  builds it and publishes it to `download.guimadaleno.com/lokal/` with
  `guimadaleno/base/release`'s action, copied into `.github/actions/publish/`.
  Read its README before
  touching `update.rs`, `install.rs`, the workflow or the key.
- **`Cargo.toml`'s version, the tag and a `## <version> — <date>` entry in
  `CHANGELOG.md` must agree;** the workflow refuses a tag otherwise. The
  entry is the release's notes, written for the person using Lokal.

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
