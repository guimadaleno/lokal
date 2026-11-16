---
paths:
  - "src/peers.rs"
  - "src/peers/**"
  - "src/tools/mail.rs"
  - "src/main.rs"
  - "src/conversation.rs"
  - "src/config.rs"
  - "src/input.rs"
  - "src/commands.rs"
---

## Let mail from another session act for the person, or run without bounds

- **`mail = auto`, the default, lets mail start a turn on its own,** as
  in Claude Code, in a session just opened too. The `>` must sit on an
  empty line: mail waits while the person types or a turn runs.
  - `mail = ask` makes every mail wait for Enter on an empty line, and
    Esc drops it.
  - `mail = off` means no socket and no tools.
- **Esc stops the exchange, not only the turn:** after the person stops
  a turn mail started, or says no in one, mail waits for their next
  request there. Otherwise the next mail would start a turn at once,
  and two models answering each other couldn't be stopped. Enter on an
  empty line passes waiting mail on sooner.
- **At most `mail_turns` (64) turns in a row start on mail,** counted
  from the person's last request; past that, mail waits the same way.
  The sender's model hears so and tells its person. Without a bound, two
  models answering each other would fill the GPU and the context.
- **The prompt and the `mail` tool steer away from loops of politeness.**
  They say to answer what asks or needs something, a plain yes or no
  included, but never mail only to thank or to say it was received.
  - A mail marked `now: false` (news, thanks) starts no turn: it goes
    along with the next one.
  - The bound is still the guard, since a prompt is only a nudge, and a
    steered model ignores it.
- **Mail never approves anything:** every step that isn't read-only
  asks the person at that terminal, as always. A turn mail started stops
  with Esc or Ctrl+C like any other.
- **The model gets mail as another session's words,** framed with its
  name and folder, never as the person's. The exception is mail the
  person wrote at the other session's `>` (`@name …`), which Lokal
  sends itself and frames as theirs.
- **What another model wrote always shows:** each mail whole, without
  control characters and with the bidi controls that reorder text shown
  by their code (`<U+202E>`), as are the characters that show as nothing
  (zero-width ones, the Tags block), and sending as a step.
  - Lokal's own facts about the other sessions go to the model without
    showing, the way its facts about the machine do: names, folders,
    branches, what each does.
  - `/sessions` shows the same facts, as `/sysprompt` and `/info` show
    the others.
- **Only the person's own sessions talk:** a socket per session,
  `<n>.sock`, beside its scratch folder in the private
  `/tmp/lokal-<uid>`, with `n` the smallest number free.
  - The kernel says who connected (`SO_PEERCRED`): the same user, and
    the very process whose socket the sender's name points to, which is
    asked back who it is.
  - Nothing that runs unasked can open a socket, so a model can't pass
    its own words off as a session's, or as the person's.
- **Conversations at a terminal only,** where someone sees mail arrive
  and can stop what it starts: a one-shot request neither listens nor
  gets the tools.
- **Bounded:** at most 2,000 characters a mail and 5 waiting in a
  session; the sender's model is told to shorten it, or that it's full.
  The socket is all it puts on disk, and it goes when the session quits,
  or at the next start if the session was killed.
