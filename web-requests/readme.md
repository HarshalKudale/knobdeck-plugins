# Web Requests

Call any web address from a key: webhooks (IFTTT, n8n, Zapier, Make, Home Assistant), REST APIs
and servers on your network. Works on Windows, macOS and Linux.

## What it does

- **HTTP Request**: send a GET, POST, PUT, PATCH, DELETE or HEAD request with optional headers and
  a JSON, form or plain-text body. Afterwards the key can show nothing (a check mark on success),
  the status code, a value from the JSON reply, or the reply's text. A failed request shows
  `HTTP 404 Not Found: <the server's message>` on the key.
- **Show Value**: a key that shows one value from a JSON reply (a price, a temperature, a build
  status, a follower count) and refreshes every 5 seconds to 15 minutes. Pressing it refreshes it
  now. Keys with the same URL share one request.
- **HTTP Toggle**: one request to switch something on and another to switch it off. Optionally
  the key reads the real state from a URL, so it follows changes made elsewhere.

URLs, headers and bodies take KnobDeck placeholders (`{selection}`, `{clipboard}`, `{url}`,
`{date}`, `{input:Message}`…). In a JSON body they are escaped for JSON, so
`{"text": "{selection}"}` stays valid whatever is selected; in a form body and in URLs they are
URL-encoded. Show Value and the toggle's state URL refresh on their own, without a key press, so
only `{date}` and `{time}` work there.

**Value paths** pick a value out of the JSON reply: names joined by dots, `[0]` for the first
item of a list, `[-1]` for the last, `["odd.key"]` for a name with dots. For
`{"data": {"items": [{"name": "a"}]}}`, `data.items[0].name` is `a`. Empty means the whole reply.

## Setup

Setup saves up to three tokens (API keys, bearer tokens) with a name each. They are kept in this
computer's credential store (macOS Keychain, Windows Credential Manager, Linux Secret Service),
never in your layout. A request picks one under **Token** and sends it as a header
(`Authorization: Bearer <token>` by default, or e.g. `X-Api-Key:`). If none of your requests
need a token, run setup with every field empty.
