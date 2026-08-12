# Using gwx from an AI agent

`gwx` is a plain CLI, so **any** shell-capable agent — Claude Code, Codex, Gemini CLI /
Antigravity (`agy`), Cursor, or your own — uses it the same way: by running `gwx` commands.
Unlike a Gemini-only Workspace extension, gwx isn't tied to one agent, and it can act on
**any** of your Google accounts via `--as` (not just the one the agent itself is signed into).

Drop the snippet below into your agent's context file (`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`,
`.cursorrules`, …) so the agent knows the tool exists and how to use it.

---

```markdown
## gwx — multi-account, policy-governed Google Workspace access

`gwx` is a local CLI (on PATH) for reading Gmail / Drive / Docs across the user's several
Google accounts. It wraps `gws` and adds: multi-account `--as`, governance (draft-only,
never-send, work accounts require human review), and cross-service link resolution.
Key point: the account you access is decoupled from the agent's own identity — use `--as`.

Commands:
- `gwx accounts`                         — list configured account names (source of truth)
- `gwx mail  --as <acct> list [N]`       — unread summary
- `gwx mail  --as <acct> list --query "travel OR 訂房 OR flight"`  — server-side Gmail search
                                            (prefer this for semantic filtering — cheaper than
                                            reading N messages and filtering yourself)
- `gwx mail  --as <acct> read <id>`      — read one message
- `gwx drive --as <acct> ls [query]`     — list Drive files (query e.g. mimeType='application/vnd.google-apps.document')
- `gwx doc   --as <acct> get <fileId>`   — export a Doc/Sheet/Slide as text (to stdout)
- `echo "<email body>" | gwx doc --as <acct> resolve`  — extract Google links in text and export their content

Governance: gwx only creates drafts and never calls a send endpoint; `gwx mail … send` is
refused; drafts on work accounts print a "🔴 REVIEW REQUIRED" notice. Never try to bypass this.
```

---

## Notes

- Run `gwx accounts` first to learn the real account names (the user chooses them, e.g.
  `work`, `personal`).
- First-time setup is `gwx auth <name>` — that opens a browser and the **human** completes the
  consent. An agent may run the command, but must not attempt to complete sign-in itself.
- Local by default; `GWX_HOST` switches to the remote/fleet mode transparently — the commands
  are identical either way.
