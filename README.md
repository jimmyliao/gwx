# gwx

**English** | [繁體中文](README.zh-TW.md)

**Multi-account, policy-governed Google Workspace access — built for AI agents (and the humans who supervise them).**

You have many Google identities — personal Gmail, work Google Workspace, side projects. Your AI agents (Claude, Codex, and friends) need to read your mail and Drive to actually help — but you don't want tokens scattered across machines, and you *definitely* don't want an agent quietly emailing your boss.

`gwx` gives your agent one clean way to work across all your Google accounts, with the dangerous actions kept behind a human. It's **local-first** — on a single machine it's just install, sign in, go — and it **scales to a fleet** when you have more than one.

```sh
gwx auth personal                            # opens Google sign-in (read + compose only, never send)
gwx mail  --as personal list                 # unread summary
gwx mail  --as work     read  <id>           # read a message
gwx drive --as work     ls "name contains 'Q3'"
gwx doc   --as work     get   <fileOrLink>   # pull the text behind a Doc/Sheet/Slide link
gwx mail  --as work     draft --to a@b.com --subject "..." --body "..."   # creates a DRAFT — never sends
```

Run bare `gwx` and it tells you what to do next — sign in if you haven't, or how to use it if you have.

## Why gwx

- **Local-first, fleet-ready.** On one machine, credentials and everything else stay local — no servers, no config. Point `GWX_HOST` at a host and the same CLI becomes a thin client for a shared credential host (see [Modes](#modes)). You only meet that complexity if you ask for it.
- **Switch accounts with one flag.** `--as <account>` — no logging in and out, no juggling browser profiles.
- **Governance built in.** Draft-only by default — gwx only ever creates drafts and **never calls a send endpoint** (and doesn't request the `gmail.send` scope). Work identities require you to review a draft before anything happens with it. Autonomous sending is never allowed.
- **Cross-service, not just mail.** Open an email, follow the Google Doc / Sheet / Drive link inside, and read the content behind it — one flow.
- **Works with any shell agent.** It's a plain CLI, so Claude / Codex / agy / your terminal all use it the same way. (An MCP interface is on the roadmap.)

## How it works

`gwx` wraps [`gws` (the Google Workspace CLI)](https://github.com/googleworkspace/cli) as its engine — so it inherits the full Workspace API surface — and adds the four things `gws` alone doesn't:

1. **Multi-account routing** (`--as`)
2. **Policy governance** (review gates, no-send, per-identity rules)
3. **Cross-service link resolution** (mail → doc/sheet/drive content)
4. **Local-first, with an optional scoped fleet mode** (below)

## Modes

**Local (default).** Everything runs on the machine you're on — gws and your credentials live locally, nothing to host, no network. This is all a single-machine developer ever needs:

```sh
gwx auth personal            # sign in once
gwx mail --as personal list  # go
```

**Remote / fleet (opt-in).** Have several machines and want your agents on all of them to share one set of credentials? Set `GWX_HOST` to a host you control. Now the tokens live only on that host, every other machine runs `gwx` as a **thin client** that asks it to perform a capability over your own private network — and never holds a token itself. Same commands, same governance; you just opted into the topology. Most people never set `GWX_HOST`.

## Install

> **Status: prototype.** The commands below describe the **target install UX**. gwx is currently a thin CLI dogfooded against a self-hosted backend — see [`docs/SPEC.md`](docs/SPEC.md). The one-liner and packages land with the first tagged release.

```sh
# Quick install (target UX — auto-detects your platform)
curl -fsSL https://raw.githubusercontent.com/jimmyliao/gwx/main/install.sh | sh
```

Planned channels, once releases ship:

| Channel | Command | For |
|---|---|---|
| **curl \| sh** | `curl -fsSL .../install.sh \| sh` | most people — one line, no toolchain |
| **Homebrew** | `brew install jimmyliao/gwx/gwx` | macOS / Linuxbrew |
| **Cargo** | `cargo install gwx` (or `cargo binstall gwx`) | Rust users; binstall skips compiling |
| **Manual binary** | download from [Releases](https://github.com/jimmyliao/gwx/releases) | CI, air-gapped, custom |

Release binaries cover `{x86_64,aarch64}-{apple-darwin, unknown-linux-gnu, unknown-linux-musl}` (musl included so Alpine/Docker work) via [cargo-dist](https://github.com/axodotdev/cargo-dist) — the same pipeline `gws` and `uv` use, which also generates the install script and Homebrew formula. The release workflow is in place and verified locally (`dist plan` / `dist build` produce all six targets, the installer, and the formula); the channels above go live on the first tagged release.

Windows (`x86_64-pc-windows-msvc`, with an auto-generated `install.ps1`) is a one-line addition to the target list — deferred until there's a Windows host to test on.

## Safety model

- **Draft-only by default** — gwx authorizes read + compose (never `gmail.send`) and its code only ever calls `drafts.create`, never a send endpoint. (Honest note: the compose scope *can* technically send, so the guarantee is gwx's code, not the scope alone.)
- **Work identities are review-gated** — creating a draft on a work account surfaces a mandatory "review required" notice; the task isn't done until you've looked.
- **No autonomous send, ever** — sending requires an explicit human confirmation and a separately-granted scope.

## Status

Early prototype, dogfooding in progress. Private while it stabilizes; the core is intended to become open source. Part of **LeapCore**.

## License

[Apache-2.0](LICENSE) — the same license as the `gws` engine it builds on. Includes an explicit patent grant.
