---
paths:
  - "site/**"
  - "tools/**"
  - "README.md"
  - "Cargo.toml"
---

# The page and the README

- **Credits the product carries** (`/about`, the page, `Cargo.toml`): the author and https://guimadaleno.com, as `Cargo.toml`'s `authors` and `homepage` say. The repository is public: `Cargo.toml` names it, and the page links to it under the install line and in the footer.
- **The page and `README.md` both say how to set up each server and what Lokal won't do:** a change to a server's setup or to the refusals changes both. The page's setup tabs name the servers' own screens and options: when a server changes them, check those words.- **The icons come from `python3 tools/icon.py`,** never from editing the files: the SVG, the PNG and every size of the `.ico` share its numbers, and the smallest size has a drawing of its own.
- **A page that says what a version brings is pushed after its tag's run:** a push to `site/` publishes the page, which offers the version `latest.json` names.
