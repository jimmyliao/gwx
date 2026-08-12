# gwx

[English](README.md) | **繁體中文**

**多帳號、有治理的 Google Workspace 存取層 —— 為 AI agent(以及在旁監督的人）打造。**

你有一堆 Google 身分 —— 個人 Gmail、公司 Google Workspace、各種副專案。你的 AI agent（Claude、Codex 等）要真的幫得上忙,就得讀你的信和雲端硬碟 —— 但你不會想讓 token 散落在每一台機器上,更*絕對*不想讓某個 agent 默默替你寄信給老闆。

`gwx` 讓你的 agent 用同一套乾淨的方式操作你所有的 Google 帳號,危險動作一律留給人。它是 **local-first** —— 一台電腦上就是裝好、登入、開跑 —— 而且**能長成 fleet**,等你機器不只一台時。

```sh
gwx auth personal                            # 開 Google 登入(只 read + compose,永不寄信)
gwx mail  --as personal list                 # 未讀摘要
gwx mail  --as work     read  <id>           # 讀一封信
gwx drive --as work     ls "name contains 'Q3'"
gwx doc   --as work     get   <fileOrLink>   # 拉出 Doc/Sheet/Slide 連結背後的文字
gwx mail  --as work     draft --to a@b.com --subject "..." --body "..."   # 只建【草稿】—— 永不寄出
```

直接打 `gwx`(無參數)它會告訴你下一步 —— 還沒登入就教你登入,登入了就教你怎麼用。

## 為什麼用 gwx

- **Local-first、可長成 fleet。** 一台電腦上,憑證與一切都留在本地 —— 沒有伺服器、沒有設定。把 `GWX_HOST` 指向一台主機,同一個 CLI 就變成共用憑證主機的**瘦客戶端**(見 [模式](#模式))。你不主動要求,就碰不到那份複雜度。
- **切帳號只要一個旗標。** `--as <account>` —— 不用登入登出,不用在瀏覽器多重設定檔之間切換。
- **治理內建。** 預設只建草稿 —— gwx **只呼叫 drafts.create、永不呼叫任何 send**(也不請求 `gmail.send` scope)。工作身分建草稿後,一定要你先過目才算數。自主寄信永遠不允許。
- **跨服務,不只是信。** 開一封信、跟著裡面的 Google Doc / Sheet / Drive 連結,直接讀到連結背後的內容 —— 一氣呵成。
- **任何 shell agent 都能用。** 它就是個純 CLI,所以 Claude / Codex / agy / 你的終端機用法完全一致。(MCP 介面在 roadmap 上。)

## 運作方式

`gwx` 以 [`gws`(Google Workspace CLI)](https://github.com/googleworkspace/cli) 當引擎(wrap)—— 因此繼承了完整的 Workspace API 面 —— 再補上 `gws` 單獨做不到的四層:

1. **多帳號路由**（`--as`）
2. **政策治理**（review 關卡、禁止寄信、逐身分規則）
3. **跨服務連結穿透**（信 → doc/sheet/drive 內容）
4. **Local-first,外加可選的 scoped fleet 模式**（見下）

## 模式

**本地(預設)。** 一切都在你當下這台跑 —— gws 與憑證都在本地,不用架主機、不走網路。單機開發者需要的就這些:

```sh
gwx auth personal            # 登入一次
gwx mail --as personal list  # 開跑
```

**遠端 / fleet(選用)。** 有好幾台機器、想讓每台上的 agent 共用同一組憑證?把 `GWX_HOST` 設成一台你掌控的主機。現在 token 只留在那台,其他每台的 `gwx` 都是**瘦客戶端**,透過你自己的私有網路請它執行能力 —— 自己絕不持有 token。指令一樣、治理一樣,只是你**主動**選了這個拓撲。多數人永遠不會設 `GWX_HOST`。

## 安裝

> **狀態:雛形。** 下面的指令描述的是**目標安裝體驗**。gwx 目前是對自架後端 dogfood 的薄 CLI —— 見 [`docs/SPEC.md`](docs/SPEC.md)。一行安裝與各套件管道會隨第一個 tagged release 上線。

```sh
# 一行安裝（目標體驗 —— 自動偵測你的平台）
curl -fsSL https://raw.githubusercontent.com/jimmyliao/gwx/main/install.sh | sh
```

release 上線後規劃的管道:

| 管道 | 指令 | 適合 |
|---|---|---|
| **curl \| sh** | `curl -fsSL .../install.sh \| sh` | 大多數人 —— 一行,不需工具鏈 |
| **Homebrew** | `brew install jimmyliao/gwx/gwx` | macOS / Linuxbrew |
| **Cargo** | `cargo install gwx`(或 `cargo binstall gwx`) | Rust 使用者;binstall 免編譯 |
| **手動 binary** | 從 [Releases](https://github.com/jimmyliao/gwx/releases) 下載 | CI、離線環境、自訂 |

release binary 涵蓋 `{x86_64,aarch64}-{apple-darwin, unknown-linux-gnu, unknown-linux-musl}`(含 musl,讓 Alpine/Docker 能跑),透過 [cargo-dist](https://github.com/axodotdev/cargo-dist) 產生 —— 與 `gws`、`uv` 同一套 pipeline,連安裝腳本與 Homebrew formula 都一併生成。release workflow 已就位並在本地驗證通過(`dist plan` / `dist build` 產出全部六個 target、installer 與 formula);上述管道會隨第一個 tagged release 生效。

Windows(`x86_64-pc-windows-msvc`,附自動生成的 `install.ps1`)只是 target 清單多加一行 —— 等有 Windows 主機可測時再補。

## 安全模型

- **預設只建草稿** —— gwx 只授權 read + compose(不含 `gmail.send`),且程式層只呼叫 `drafts.create`、永不碰 send。(誠實說明:compose scope 本身**技術上能寄**,所以保證來自 gwx 的程式碼,不是 scope。)
- **工作身分強制 review** —— 在工作帳號建草稿會跳出強制的「review required」提示;你沒看過就不算完成。
- **永不自主寄信** —— 要寄信需要人明確確認,外加另外授權的 scope。

## 狀態

早期雛形,dogfooding 進行中。穩定前先保持 Private;核心預計開源。屬於 **LeapCore**。

## 授權

[Apache-2.0](LICENSE) —— 與所 wrap 的 `gws` 引擎同授權,內含明確專利 grant。
