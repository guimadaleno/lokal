---
name: measure
description: "Use before and after a change that could move Lokal's size, start time or memory, auto's judge's time, or a redraw at a new width: what to measure, and the page's figures a change may make false."
---

# Measure

Measure before and after the change, on the same machine:

1. **Size:** the release build (static, stripped), as `release.yml` builds it.
2. **Start:** from launch to the prompt, the server found.
3. **Memory** at the prompt.
4. **Auto's judge:** the time a step takes, reasoning off and on, and that the conversation's prompt cache stays.
5. **A redraw at a new width:** the release build in a pseudo-terminal, for a short conversation and for the most it keeps (`MOST_KEPT`), and from the resize to the redraw.

The page's *Light* paragraph (`site/index.html`) quotes the start time and the memory, and fills the size from the release: a change that moves past them changes those words too.
