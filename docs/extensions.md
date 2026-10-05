# Extensions and Pipkin: what is supported

Extension code runs in Pi, never in Pipkin. An extension that wants to ask the person something does it through the engine, and Pipkin shows it. This page says exactly which interactions exist, which do not, and how an unsupported one fails.

## The contract (Pi, in the fork `../pi-fork/pi`)

Two session-scope pieces, in `packages/coding-agent/src/experimental/services/`:

- **`UiRequests`** (`pi.ui-requests`, remote): replicated state `{ requests, status, notices }`, and `respond(id, value)` and `cancel(id)`. A presentation subscribes to it, shows the questions, and answers by request id.
- **`UiRequestHost`** (local, for other facets in the Session worker): `select`, `confirm`, `input`, `notify`, `setStatus`. Each question returns a promise that settles when a presentation answers, when it is cancelled, when it times out (two minutes unless the extension says otherwise; `0` waits indefinitely), when the extension's own abort signal fires, or when the Session shuts down. An unanswered question resolves `undefined`, so an extension never waits forever on a presentation that cannot ask.

A working example is `packages/coding-agent/examples/plugins/pi-example-questions`. Start Pipkin with `--pi-extension <that folder>` and creating a conversation shows its three questions.

## What Pipkin shows

| Interaction | Supported | How it appears | If it cannot be shown |
| --- | --- | --- | --- |
| Choose one of several options (`select`) | Yes | A dialog with the choices and their descriptions; up/down and Enter, or a click | n/a |
| Yes or no (`confirm`) | Yes | A dialog with Yes and No | n/a |
| A line of text (`input`) | Yes | A dialog with a text field, filled with the extension's default | n/a |
| A notice (`notify`) | Yes | A strip above the conversation with the latest three messages, dismissible | n/a |
| A status line (`setStatus`) | Yes | One quiet line under the conversation header, `key: text` | n/a |
| A question of any other kind (an editor, a multi-select, a custom form) | No | A warning notice naming the question and saying this version cannot show it | The question is not shown; the extension's own timeout ends it |
| Terminal widgets, custom TUI components, key bindings and anything else drawn by the terminal presentation | No | Nothing: these are terminal-only and are not part of the remote contract | The extension's TUI facet does not run in Pipkin |
| Slash commands contributed by a plugin's TUI facet (`/hello`) | No | Not listed: `SlashCommands` is a process-local hookpoint of the terminal presentation | n/a |
| Extensions written for the stable Pi extension API (`ctx.ui.select` and friends) | No | They are not routed to this host yet, so they cannot ask through Pipkin | n/a |

Everything that **is** supported follows the same rules:

- Each question has the engine's own id. An answer is sent once, and the question leaves the screen only when the engine's state says it is over, so a refused answer (a choice that is not on offer, a question that already ended) keeps the question open and shows the engine's reason.
- Dismissing a dialog (Escape, or a click outside) only puts it aside: a strip says an extension is waiting, with Answer and Decline. Declining tells the extension there was no answer.
- A deadline the engine set is shown ("Cancels itself in about 2 min"); when it passes the engine cancels the question and says so in a notice.
- When the connection to the engine drops, no question is offered as answerable. After reconnecting, whatever the engine still holds is shown again.
- No more than twenty questions are open at once; the extension is told no for the rest and a notice says why.
- The palette has "Answer the extension's question", "Decline the extension's question" and "Dismiss notices from extensions", so none of it needs a pointer.

## What is not done

Routing the stable extension `ctx.ui.*` API into the host, so existing extensions ask through any presentation, is a Pi change that is not made here. Until it is, only facets written for the experimental Session worker (like the example) can ask.
