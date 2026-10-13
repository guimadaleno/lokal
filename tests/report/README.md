# Test vectors

What every copy of the client, and the service, must make of a field, a
report and an answer. `client/report.rs` runs them in each desktop app that
carries it (as `tests/report/`), `client/report.js` runs them here, and the
Worker applies `client/report.js` itself.

They are written by `tools/vectors.mjs`, which writes the same bytes every
time; `npm test` checks that. Every file is ASCII: a character outside it is
spelled `\uXXXX`, because several of them are invisible. An app copies this
folder into its own tests; a change here is a change every copy takes.

| File | Each entry | Must give |
|---|---|---|
| `clean.json` | `as` (`line` or `text`) and `from` | `to`, and `to` again when cleaned twice |
| `reports.json` | `name` and a `report` any client can build | `verdict`: `null` when it can be sent, or the first field at fault |
| `shapes.json` | `name` and a `report` only JSON can hold (a wrong type, a missing or unknown field) | `verdict`, the same way |
| `answers.json` | `status` and `body`, as the service or anything in between might answer | `outcome`: `sent` with its `reference`, `invalid` with its `field`, `later` with its `seconds`, or `refused` |

`shapes.json` is for the service and for a page, whose reports are plain
objects. A desktop client's types cannot spell those reports, so it does not
run them. One vector in `reports.json` it reads differently: details with
nothing in them are, to its types, no details, and that can be sent.

Lengths are in code points. The two vectors with a title of 120 and of 121
emoji are what holds every client to that: counted in UTF-16 units or in
bytes, both would be refused.
