# AI Actions

Fix, rewrite, translate, summarize or explain the selected text with your own AI model, and put
the answer back in place. Works on Windows, macOS and Linux.

## What it does

- **AI Action**: one key that runs one of the presets on the selected text, or your own prompt.
  Presets: Fix Grammar, Rewrite Clearly, Make Shorter, Make Professional, Make Friendly, Bullet
  Points, Translate (into the language you choose), Summarize, Explain, Explain Code, Draft a
  Reply.
- **AI Actions Wheel**: every preset on a wheel; pick one for the selected text.

Where the answer goes: **in place of the selected text**, **on the clipboard**, or **in a
notification** (and on the clipboard). By default, edits (fix, rewrite, translate…) replace the
text and answers (summarize, explain, reply) are shown.

When nothing is selected, the clipboard's text is used instead (a checkbox turns this off). Your
own prompt can place the text itself with `{selection}` or `{clipboard}`; a prompt without them
works on the selected text, or, with nothing selected, is sent as it is. `{input:Ask AI}` asks
you for a question in a small box, so `{input:Ask AI}` alone is a quick question key.

While the model answers, the key shows a spinner.

## Setup

Choose a provider and paste your own key:

- **Anthropic (Claude)**: a key from console.anthropic.com. The default model is
  `claude-opus-5-5` (asked to answer with low effort, so edits come back quickly); set any other
  model id, e.g. `claude-haiku-5-5`, in setup.
- **OpenAI or an OpenAI-compatible server**: an OpenAI key (default model `gpt-5`), or the address
  of another server that speaks the same API: LM Studio (`http://localhost:1234/v1`), OpenRouter,
  Groq… The key is optional for local servers.
- **Ollama** (on this computer): no key. Leave the model empty to use your first installed model.

Setup checks that the key works and that the model exists before saving. The key is kept in this
computer's credential store, never in a file.

## How it works

The selected text is read with KnobDeck's selection reader (UI Automation, Accessibility, AT-SPI;
never by sending Ctrl+C). The plugin calls the provider's API directly: Anthropic's Messages API,
the Chat Completions API, or Ollama's `/api/chat`.

Entering the answer is the one place this plugin produces input, on purpose (its job is entering
text, like Text Snippet): on Windows and Linux the answer is put on the clipboard and pasted with
Ctrl+V, so a long answer arrives at once and line breaks don't press Enter in chat apps; on macOS
it is typed as Unicode key events, line breaks as ⇧Return (this needs KnobDeck in System Settings
▸ Privacy & Security ▸ Accessibility). If entering fails, the answer is left on the clipboard.
