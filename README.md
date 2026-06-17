# cc-mapping

Language | 语言: [English](#english) · [中文](#中文)

## English

**A lightweight HTTP proxy for Claude Code and Codex CLIs.**

`cc-mapping` sits between your AI CLI tools and upstream APIs. It routes each request by the `model` field in the request body, forwarding to the matching `apiUrl` with the configured `apiKey`.

## Key Features

  * **Model-aware routing**: Route each model to a different upstream via `model_mapping`.
  * **Optional model rename**: Replace the request `model` field before forwarding (e.g. map `deepseek-v3` to `deepseek-v4-pro`).
  * **Auto-Configuration**: Automatically manages the proxy settings for Claude Code and Codex CLIs—no manual export needed.
  * **Lightweight**: A single Rust binary with no database or heavy dependencies.

-----

## 🛠️ Installation

### Pre-built Binaries (Recommended)

Download the latest release for your platform from the [Releases](https://github.com/Lorem3/cc-mapping/releases) page.

```bash
# Linux x86_64
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-x86_64-unknown-linux-gnu.tar.gz
tar xzf cc-mapping-x86_64-unknown-linux-gnu.tar.gz
sudo mv cc-mapping /usr/local/bin/

# Linux arm64
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-aarch64-unknown-linux-gnu.tar.gz
tar xzf cc-mapping-aarch64-unknown-linux-gnu.tar.gz
sudo mv cc-mapping /usr/local/bin/

# macOS arm64 (Apple Silicon)
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-aarch64-apple-darwin.tar.gz
tar xzf cc-mapping-aarch64-apple-darwin.tar.gz
sudo mv cc-mapping /usr/local/bin/

# macOS x86_64 (Intel)
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-x86_64-apple-darwin.tar.gz
tar xzf cc-mapping-x86_64-apple-darwin.tar.gz
sudo mv cc-mapping /usr/local/bin/
```

Windows users can download `cc-mapping-x86_64-pc-windows-msvc.zip` from the Releases page and place `cc-mapping.exe` in a directory on your `PATH`.

### Prerequisites

  * **Rust**: Ensure you have `cargo` installed.

### Build from Source

```bash
# Clone the repository
git clone https://github.com/Lorem3/cc-mapping.git
cd cc-mapping

# Build release binary
cargo build --release

# Install globally
sudo cp target/release/cc-mapping /usr/local/bin/
```

### Publishing a New Release

Update the `version` field in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions will automatically build binaries for all five platforms and publish a GitHub Release.

-----

## 🚀 Usage

### Basic Commands

```bash
# Start the proxy (daemon mode)
# This automatically configures Claude & Codex to use the proxy.
cc-mapping start

# Start with request logging (prints incoming URL, upstream URL, and bodies)
cc-mapping start -log

# Check connection status and current routing
cc-mapping status

# Reload provider.json without restarting (sends SIGUSR1 to the daemon)
cc-mapping reload

# Print and open provider.json with the system default editor
cc-mapping config

# Stop the proxy
cc-mapping stop
```

`cc-mapping` listens on `0.0.0.0:18100` by default and automatically detects your LAN IP.
Share the reported URL (for example `http://192.168.1.252:18100`) with other machines
so their CLIs can reuse the same proxy and model_mapping configuration.

### Hot-reload

The daemon automatically watches `~/.cc-mapping/provider.json` for changes and reloads
the `model_mapping` configuration on every save—no restart required. You can also
trigger a manual reload at any time:

```bash
cc-mapping reload
```

### Sub-path routing

The proxy forwards the full request path to the upstream, so sub-paths work automatically.
For example, `POST /v1/messages/count_tokens` is routed the same way as `POST /v1/messages`.

### Machine B (remote CLI) example

When **Machine A** runs `cc-mapping start` and shows `Share this URL: http://192.168.0.10:18100`,
you can point **Machine B**'s CLI tools to that proxy without running another daemon.
Create these minimal config files on Machine B (replace the IP with the one reported by Machine A):

**`~/.claude/settings.json`**

```json
{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "cc-mapping",
    "ANTHROPIC_BASE_URL": "http://192.168.0.10:18100"
  }
}
```

**`~/.codex/config.toml`**

```toml
preferred_auth_method = "apikey"
model = "gpt-5-codex"
model_provider = "cc-mapping"

[model_providers.cc-mapping]
name = "cc-mapping"
base_url = "http://192.168.0.10:18100"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
```

**`~/.codex/auth.json`**

```json
{
  "OPENAI_API_KEY": "cc-mapping"
}
```

### Configuration

Create your configuration file at `~/.cc-mapping/provider.json`.

#### Model Mapping

Use `model_mapping` to route each model to a different upstream. When the request's `model` field contains a key (case-insensitive substring match), the proxy forwards to that entry's `apiUrl` and injects its `apiKey`. Longer, more specific keys take priority.

Optionally set `name` to replace the entire `model` field in the request body before forwarding.

`model_mapping` supports two value formats:

- Direct object: points to `apiUrl`/`apiKey` (and optional `name`) directly.
- String alias: points to another key in the same `model_mapping`, then uses that entry's object as the final upstream config.

```json
{
  "model_mapping": {
    "provider_deepseek": {
      "apiUrl": "https://api.deepseek.com/v1",
      "apiKey": "sk-ds-key",
      "name": "deepseek-v4-pro"
    },
    "deepseek-v3": "provider_deepseek",
    "mimo-v2.5-pro": { "apiUrl": "https://api.xiaomimimo.com/anthropic", "apiKey": "sk-mimo-key" },
    "sonnet":        { "apiUrl": "https://api.anthropic.com",            "apiKey": "sk-ant-sonnet-key" }
  }
}
```

**Matching rules:**

- Match is case-insensitive substring: `"sonnet"` matches `claude-sonnet-4-5`, `claude-sonnet-3-7`, etc.
- More specific (longer) keys win: `"mimo-v2.5-pro"` matches before `"mimo-v2.5"`.
- If `name` is set, the request body's `model` field is replaced entirely before forwarding.
- If an alias value (e.g. `"provider_deepseek"`) is missing in `model_mapping`, that mapping entry is skipped (fallback behavior).
- If no key matches the model, the request fails with an error.
- **Dashscope compatibility**: When `apiUrl` contains both `dashscope` and `anthropic`, the proxy automatically uses `x-api-key` header authentication and strips `anthropic-version`/`anthropic-beta` headers for compatibility with Dashscope's Anthropic-compatible API.

-----

## 中文

**为 Claude Code 与 Codex CLI 提供的轻量 HTTP 代理。**

`cc-mapping` 位于本地 CLI 与上游 API 之间，根据请求体中的 `model` 字段，通过 `model_mapping` 将请求转发到对应的 `apiUrl`，并注入配置的 `apiKey`。

### 核心特性

  * **按模型路由**：通过 `model_mapping` 将不同 model 转发到不同上游。
  * **可选 model 替换**：转发前可将请求体中的 `model` 整字段替换为配置的 `name`。
  * **自动配置**：无需手动导出代理变量，自动配置 Claude Code 与 Codex CLI。
  * **轻量单可执行文件**：纯 Rust 实现，无数据库与重依赖。

-----

### 🛠️ 安装

#### 预编译二进制（推荐）

从 [Releases](https://github.com/Lorem3/cc-mapping/releases) 页面下载适合你平台的最新版本。

```bash
# Linux x86_64
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-x86_64-unknown-linux-gnu.tar.gz
tar xzf cc-mapping-x86_64-unknown-linux-gnu.tar.gz
sudo mv cc-mapping /usr/local/bin/

# Linux arm64
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-aarch64-unknown-linux-gnu.tar.gz
tar xzf cc-mapping-aarch64-unknown-linux-gnu.tar.gz
sudo mv cc-mapping /usr/local/bin/

# macOS arm64（Apple Silicon）
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-aarch64-apple-darwin.tar.gz
tar xzf cc-mapping-aarch64-apple-darwin.tar.gz
sudo mv cc-mapping /usr/local/bin/

# macOS x86_64（Intel）
curl -LO https://github.com/Lorem3/cc-mapping/releases/latest/download/cc-mapping-x86_64-apple-darwin.tar.gz
tar xzf cc-mapping-x86_64-apple-darwin.tar.gz
sudo mv cc-mapping /usr/local/bin/
```

Windows 用户可从 Releases 页面下载 `cc-mapping-x86_64-pc-windows-msvc.zip`，解压后将 `cc-mapping.exe` 放入 `PATH` 目录即可。

#### 先决条件

  * **Rust**：需要已安装 `cargo`。

#### 源码构建

```bash
# 克隆仓库
git clone https://github.com/Lorem3/cc-mapping.git
cd cc-mapping

# 构建发布版本
cargo build --release

# 全局安装
sudo cp target/release/cc-mapping /usr/local/bin/
```

#### 发布新版本

更新 `Cargo.toml` 中的 `version` 字段并提交，然后推送对应的 tag：

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions 将自动为全部五个平台构建二进制并发布 GitHub Release。

-----

### 🚀 使用

#### 基本命令

```bash
# 启动代理（守护模式），自动配置 Claude & Codex 代理
cc-mapping start

# 启动并开启请求日志（打印 incoming URL、upstream URL 和请求体）
cc-mapping start -log

# 查看连接状态与当前路由
cc-mapping status

# 重新加载 provider.json（无需重启，向守护进程发送 SIGUSR1 信号）
cc-mapping reload

# 打印配置文件路径并用系统默认编辑器打开
cc-mapping config

# 停止代理
cc-mapping stop
```

默认会监听 `0.0.0.0:18100` 并自动检测本机可访问的 IP。
将自动提示的地址（如 `http://192.168.1.252:18100`）分享给其他主机，即可让它们共用同一个代理与 model_mapping 配置。

#### 热重载

守护进程会自动监听 `~/.cc-mapping/provider.json` 的文件变更，每次保存后自动重载 `model_mapping` 配置，无需重启。也可以随时手动触发重载：

```bash
cc-mapping reload
```

#### 子路径路由

代理会将完整的请求路径转发到上游，因此子路径自动生效。例如 `POST /v1/messages/count_tokens` 与 `POST /v1/messages` 使用相同的路由规则。

### 机器 B（远程 CLI）示例

当 **机器 A** 执行 `cc-mapping start` 并输出 `Share this URL: http://192.168.0.10:18100` 时，
**机器 B** 可以直接将各 CLI 指向该地址，无需再额外运行代理进程。
在机器 B 上创建以下最小配置文件（记得将 IP 替换为机器 A 实际输出的地址）：

**`~/.claude/settings.json`**

```json
{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "cc-mapping",
    "ANTHROPIC_BASE_URL": "http://192.168.0.10:18100"
  }
}
```

**`~/.codex/config.toml`**

```toml
preferred_auth_method = "apikey"
model = "gpt-5-codex"
model_provider = "cc-mapping"

[model_providers.cc-mapping]
name = "cc-mapping"
base_url = "http://192.168.0.10:18100"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
```

**`~/.codex/auth.json`**

```json
{
  "OPENAI_API_KEY": "cc-mapping"
}
```

#### 配置

在 `~/.cc-mapping/provider.json` 创建配置文件。

##### 按模型路由（model_mapping）

使用 `model_mapping` 将不同 model 路由到各自的上游。当请求的 `model` 字段包含某个 key（大小写不敏感子串匹配）时，代理将转发到该 key 对应的 `apiUrl`，并注入对应的 `apiKey`。更长（更具体）的 key 优先匹配。

可选配置 `name`，在转发前将请求体中的 `model` 整字段替换为指定值。

`model_mapping` 支持两种 value 写法：

- 直接对象：直接提供 `apiUrl`/`apiKey`（可选 `name`）。
- 字符串别名：指向同一 `model_mapping` 中的另一个 key，再由该对象作为最终上游配置。

```json
{
  "model_mapping": {
    "provider_deepseek": {
      "apiUrl": "https://api.deepseek.com/v1",
      "apiKey": "sk-ds-key",
      "name": "deepseek-v4-pro"
    },
    "deepseek-v3": "provider_deepseek",
    "mimo-v2.5-pro": { "apiUrl": "https://api.xiaomimimo.com/anthropic", "apiKey": "sk-mimo-key" },
    "sonnet":        { "apiUrl": "https://api.anthropic.com",            "apiKey": "sk-ant-sonnet-key" }
  }
}
```

**匹配规则：**

- 大小写不敏感子串匹配：`"sonnet"` 可命中 `claude-sonnet-4-5`、`claude-sonnet-3-7` 等。
- 更长的 key 优先：`"mimo-v2.5-pro"` 早于 `"mimo-v2.5"` 匹配。
- 配置了 `name` 时，转发前将请求体 `model` 整字段替换为 `name`。
- 若 value 为别名（如 `"provider_deepseek"`）但 `model_mapping` 中不存在该 key，则该映射项会被跳过（回退行为）。
- 若无匹配 key，请求返回错误。
- **Dashscope 兼容**：当 `apiUrl` 同时包含 `dashscope` 和 `anthropic` 时，代理自动使用 `x-api-key` 头认证，并剥离 `anthropic-version`/`anthropic-beta` 头以兼容 Dashscope 的 Anthropic 兼容 API。

-----

## License

MIT
