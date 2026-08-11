# gwx — multi-account, policy-governed Google Workspace layer for AI agents

**狀態**：prototype（dogfood 驗證中,2026-08-11 起）。**Private**（含 fleet 內部資訊;產品方向=先私有驗證→之後抽乾淨 OSS 核心）。
**定位**：多帳號 × 有治理 × 跨服務的 Google Workspace 存取層 for AI agents。掛 LeapCore。

## 策略：wrap `gws`(googleworkspace/cli)當引擎 + 疊 4 層價值(護城河)
1. **多帳號 `--as <account>`**（路由到 per-account gws creds）
2. **policy 治理**（leap* = 強制 review + no-send;draft-only 用 readonly+compose scope 硬擋;send confirm-only）
3. **跨服務連結穿透**（信 → doc/sheet/drive ID → gws fetch/export → text）
4. **scoped serve over Tailscale**（token 只在 anchor,client 零 token/shell）

引擎分工:Drive 仍用 rclone(serve/mount 已動);gws 補 gmail+docs/sheets/slides+連結穿透。gws pre-v1.0 → pin+可 fork。

## 統一 CLI
`gwx mail --as leapdesign list` · `gwx drive --as sjliao ls` · `gwx doc --as leapcore get <link>` · `gwx resolve <link>`

## 帳號（待 OAuth）
account-a@example.com · account-b@example.com · account-c@example.com · account-d@example.com

## 形式路線
CLI(自己 fleet)→ MCP(給別人,通用語)→ Rust 單 binary。OSS 核心 + policy 商業層(EntryDesk 模式)。

*設計權威來源:~/.agents/personal/technical/anchor-connector-portal.md §11 / fleet-identity-and-tools.md*
