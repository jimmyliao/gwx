# gwx — Spec & Guideline

> What gwx is, how it's built, and the rules it enforces. For the pitch and quickstart, see the [README](../README.md).

## 1. Problem

One person, many Google identities (personal Gmail, work Workspace, side projects). AI agents running on several machines need to read that mail and Drive to be useful. Doing this naively means: OAuth tokens copied onto every laptop, no consistent account switching, and no guardrail stopping an agent from sending mail it shouldn't. gwx solves those three at once.

## 2. Design in one line

**gwx = [`gws`](https://github.com/googleworkspace/cli) as engine + four layers gws doesn't provide.**

`gws` is a Rust CLI that auto-discovers the entire Google Workspace API surface (Gmail, Drive, Docs, Sheets, Slides, Calendar, …). gwx keeps that engine and wraps it:

| Layer | What it adds | Why gws alone isn't enough |
|---|---|---|
| **1. Multi-account routing** | `--as <account>` selects a credential set per call | gws points at one credential file at a time |
| **2. Policy governance** | per-identity rules: review gates, no-send, scope caps | gws is a raw API client with no policy notion |
| **3. Link resolution** | mail body → Doc/Sheet/Drive links → their text content | crosses services in one flow; gws is per-call |
| **4. Scoped access** | token stays on a trusted host; clients are thin | gws expects the token to be local |

## 3. Architecture

```
  agent / terminal            credential host                 Google
  (any machine)               (a secret store)
  ┌───────────┐  private net  ┌────────────────────┐   OAuth   ┌─────────┐
  │  gwx CLI  │ ────────────▶ │ scoped service     │ ────────▶ │ Google  │
  │ (thin)    │  capability   │  → gws → secret get │  tokens   │Workspace│
  └───────────┘   only        │    per account      │           └─────────┘
                              └────────────────────┘
```

> The topology below is one **reference deployment** — your own may differ. The only hard requirements are: credentials live on a host *you* control, and clients reach it over a private network, never holding the token themselves.

- **Credential host**: one always-on host you control acts as a **secret store** for every account's OAuth credentials. Tokens never leave it. The store sits behind a `secret get <account>` interface, so the backend is pluggable — access-controlled files today; a real secret manager (Vault / OpenBao) later — without changing the service.
- **Transport**: your own private network — e.g. a WireGuard mesh such as [Tailscale](https://tailscale.com). Only your machines can reach the host.
- **Client**: `gwx` on each machine is thin — it asks the host to perform a *capability*, and never holds a token or a shell there.

> **Prototype note:** the current prototype reaches the host over `ssh` for speed of iteration. The product target is a **scoped service** so a client gets exactly one capability and no shell.

## 4. Accounts & the `--as` model

Each account is a named entry mapping to a credential file and a policy. You address an account explicitly per call:

```sh
gwx mail  --as work     list
gwx drive --as personal ls
```

Explicit addressing (not an ambient "current account") is deliberate: an agent can never act on the wrong identity by forgetting to switch.

## 5. Policy model (the guardrail)

Policy is per-identity and enforced two ways — by rule *and* by OAuth scope, so a rule bug can't grant a capability the token physically lacks.

- **Draft-only by default.** Accounts are authorized with `gmail.readonly` + `gmail.compose` + `drive.readonly`. Compose can create a draft; it **cannot send**. Sending would require a separately-granted `gmail.send` scope that the default setup never requests.
- **Work identities are review-gated.** Creating a draft on a work account (e.g. a `leap*` identity) raises a mandatory "🔴 review required" notice. The task is not complete until a human has looked at the draft.
- **No autonomous send, ever.** Sending is `autonomous_send: never` — it requires an explicit human confirmation, on top of the extra scope.

See [`policy.example.yaml`](../policy.example.yaml) for the schema — a redacted template. Real account emails live only in your private deployment (copy the example, fill it in, and keep it out of git).

## 6. Command surface

```
gwx mail  --as <acct> list [N]        # unread summary        (gws gmail +triage)
gwx mail  --as <acct> read <id>       # read a message        (gws gmail +read)
gwx mail  --as <acct> draft ...       # create a DRAFT — never sends; work acct → review gate
gwx drive --as <acct> ls [query]      # list files            (gws drive files list)
gwx doc   --as <acct> get <id|link>   # export Doc/Sheet/Slide as text (link resolution)
gwx accounts                          # list configured accounts
```

## 7. Scopes summary

| Scope | Grants | In default setup |
|---|---|---|
| `gmail.readonly` | read mail | ✅ |
| `gmail.compose` | create drafts (not send) | ✅ |
| `drive.readonly` | read Drive + export Docs | ✅ |
| `gmail.send` | send mail | ❌ (opt-in, per-confirmation only) |

## 8. Roadmap

- Replace the ssh-shim prototype with a scoped service on the credential host.
- Harden the secret store behind `secret get`: encrypted-at-rest (age/sops) as a light step, then Vault / OpenBao when the operational cost is justified.
- RFC822 draft assembly helper (compose real drafts through `gws gmail users drafts create`).
- Link resolution across Docs / Sheets / Slides / Drive from mail bodies.
- MCP interface alongside the CLI.
- Release pipeline (cargo-dist) → the install channels in the README.
