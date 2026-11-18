# GitHub

Keep an eye on GitHub from KnobDeck, on Windows, macOS and Linux.

## What it does

- **Notifications**: your unread count (optionally only threads you take part in); press opens
  github.com/notifications.
- **Review Requests** and **My Open PRs**: open pull requests waiting for your review, and your own;
  press opens the matching github.com/pulls page.
- **Workflow Status**: the latest run of a repository (or one workflow, optionally on one branch) as
  the key's state (running, success, failed, cancelled); press opens the run.
- **Re-run Failed Jobs**: re-runs the failed jobs of that latest run.
- **Run Workflow**: starts a workflow that has a `workflow_dispatch` trigger, on a branch or tag
  (the default branch when empty), with `name=value` inputs (placeholders such as `{date}` work).
- **Repository Counter**: stars, open issues or open pull requests of any repository; press opens
  the matching page.

## Setup

Setup opens a page with a code: press **Copy code and open GitHub**, paste the code on
github.com/login/device and approve KnobDeck. KnobDeck asks for the `repo` and `notifications`
permissions.

You can paste a personal access token instead (Settings ▸ Developer settings ▸ Personal access
tokens). A fine-grained token needs the repository permissions Actions (read and write), Pull
requests, Issues and Metadata (read). GitHub doesn't let fine-grained tokens read notifications, so
for the Notifications key use a classic token with the `notifications` and `repo` scopes.

## How it works

GitHub's REST API (and GraphQL for the repository counters) with your token, kept in this
computer's credential store (macOS Keychain, Windows Credential Manager, Linux Secret Service).
Keys refresh politely: replies are asked again with their ETag (unchanged answers don't count
against your rate limit), GitHub's notification poll interval is honoured, and when the rate limit
runs out the keys keep their last value until it resets.
