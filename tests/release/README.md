# Test vectors

Every app's updater opens each of these with `test.pub` as its public key and
`example` as its own name, and must reach the verdict below. They are written by
`tools/vectors.py`, which writes the same bytes every time; `tests/` checks both.

The key that signed them is public on purpose (its seed is the bytes 0 to 31)
and signs nothing else. An app copies this folder into its own tests; a change
here is a change every copy takes.

| File | Verdict | Why |
|---|---|---|
| `valid.json` | accept: `1.2.0`, one file `example.exe`, two downloads | |
| `prerelease.json` | accept: `1.2.0-beta.2` | pre-releases are versions |
| `tampered.json` | refuse: signature | one digit of a size changed after signing |
| `other-app.json` | refuse: app | signed by the right key, for `other` |
| `no-app.json` | refuse: app | signed, with no `app` at all |
| `plain-http.json` | refuse: contents | a file's URL is `http://` |
| `unknown-file.json` | refuse: contents | `setup.bat` is on no client's list of files |
| `short-signature.json` | refuse: malformed | the signature is 32 bytes, not 64 |

For a client, "refuse" is all that matters, and the reason is for its tests'
messages. `unknown-file.json` is a valid release as far as the publisher is
concerned: refusing it is the client's own list of the files it installs.
