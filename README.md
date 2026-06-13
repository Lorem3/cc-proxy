# cc-proxy

Language | 语言: [English](#english) · [中文](#中文)

## English

**A lightweight HTTP proxy for Claude Code and Codex CLIs.**

`cc-proxy` sits between your AI CLI tools and upstream APIs. It routes each request by the `model` field in the request body, forwarding to the matching `apiUrl` with the configured `apiKey`.

## Key Features

  * **Model-aware routing**: Route each model to a different upstream via `model_mapping`.
  * **Optional model rename**: Replace the request `model` field before forwarding (e.g. map `deepseek-v3` to `deepseek-v4-pro`).
  * **Auto-Configuration**: Automatically manages the proxy settings for Claude Code and Codex CLIs—no manual export needed.
  * **Lightweight**: A single Rust binary with no database or heavy dependencies.

-----

## 🛠️ Installation

### Pre-built Binaries (Recommended)

Download the latest release for your platform from the [Releases](https://github.com/Lorem3/cc-proxy/releases) page.

```bash
# Linux x86_64
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-x86_64-unknown-linux-gnu.tar.gz
tar xzf cc-proxy-x86_64-unknown-linux-gnu.tar.gz
sudo mv cc-proxy /usr/local/bin/

# Linux arm64
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-aarch64-unknown-linux-gnu.tar.gz
tar xzf cc-proxy-aarch64-unknown-linux-gnu.tar.gz
sudo mv cc-proxy /usr/local/bin/

# macOS arm64 (Apple Silicon)
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-aarch64-apple-darwin.tar.gz
tar xzf cc-proxy-aarch64-apple-darwin.tar.gz
sudo mv cc-proxy /usr/local/bin/

# macOS x86_64 (Intel)
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-x86_64-apple-darwin.tar.gz
tar xzf cc-proxy-x86_64-apple-darwin.tar.gz
sudo mv cc-proxy /usr/local/bin/
```

Windows users can download `cc-proxy-x86_64-pc-windows-msvc.zip` from the Releases page and place `cc-proxy.exe` in a directory on your `PATH`.

### Prerequisites

  * **Rust**: Ensure you have `cargo` installed.

### Build from Source

```bash
# Clone the repository
git clone https://github.com/arhsis/cc-proxy.git
cd cc-proxy

# Build release binary
cargo build --release

# Install globally
sudo cp target/release/cc-proxy /usr/local/bin/
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
cc-proxy start

# Check connection status and current routing
cc-proxy status

# Stop the proxy and revert CLI configurations
cc-proxy stop
```

`cc-proxy` listens on `0.0.0.0:18100` by default and automatically detects your LAN IP.
Share the reported URL (for example `http://192.168.1.252:18100`) with other machines
so their CLIs can reuse the same proxy and model_mapping configuration.

### Machine B (remote CLI) example

When **Machine A** runs `cc-proxy start` and shows `Share this URL: http://192.168.0.10:18100`,
you can point **Machine B**'s CLI tools to that proxy without running another daemon.
Create these minimal config files on Machine B (replace the IP with the one reported by Machine A):

**`~/.claude/settings.json`**

```json
{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "cc-proxy",
    "ANTHROPIC_BASE_URL": "http://192.168.0.10:18100"
  }
}
```

**`~/.codex/config.toml`**

```toml
preferred_auth_method = "apikey"
model = "gpt-5-codex"
model_provider = "cc-proxy"

[model_providers.cc-proxy]
name = "cc-proxy"
base_url = "http://192.168.0.10:18100"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
```

**`~/.codex/auth.json`**

```json
{
  "OPENAI_API_KEY": "cc-proxy"
}
```

### Configuration

Create your configuration file at `~/.cc-proxy/provider.json`.

#### Model Mapping

Use `model_mapping` to route each model to a different upstream. When the request's `model` field contains a key (case-insensitive substring match), the proxy forwards to that entry's `apiUrl` and injects its `apiKey`. Longer, more specific keys take priority.

Optionally set `name` to replace the entire `model` field in the request body before forwarding.

```json
{
  "model_mapping": {
    "deepseek-v3": {
      "apiUrl": "https://api.deepseek.com/v1",
      "apiKey": "sk-ds-key",
      "name": "deepseek-v4-pro"
    },
    "mimo-v2.5-pro": { "apiUrl": "https://api.xiaomimimo.com/anthropic", "apiKey": "sk-mimo-key" },
    "sonnet":        { "apiUrl": "https://api.anthropic.com",            "apiKey": "sk-ant-sonnet-key" }
  }
}
```

**Matching rules:**

- Match is case-insensitive substring: `"sonnet"` matches `claude-sonnet-4-5`, `claude-sonnet-3-7`, etc.
- More specific (longer) keys win: `"mimo-v2.5-pro"` matches before `"mimo-v2.5"`.
- If `name` is set, the request body's `model` field is replaced entirely before forwarding.
- If no key matches the model, the request fails with an error.

-----

## 中文

**为 Claude Code 与 Codex CLI 提供的轻量 HTTP 代理。**

`cc-proxy` 位于本地 CLI 与上游 API 之间，根据请求体中的 `model` 字段，通过 `model_mapping` 将请求转发到对应的 `apiUrl`，并注入配置的 `apiKey`。

### 核心特性

  * **按模型路由**：通过 `model_mapping` 将不同 model 转发到不同上游。
  * **可选 model 替换**：转发前可将请求体中的 `model` 整字段替换为配置的 `name`。
  * **自动配置**：无需手动导出代理变量，自动配置 Claude Code 与 Codex CLI。
  * **轻量单可执行文件**：纯 Rust 实现，无数据库与重依赖。

-----

### 🛠️ 安装

#### 预编译二进制（推荐）

从 [Releases](https://github.com/Lorem3/cc-proxy/releases) 页面下载适合你平台的最新版本。

```bash
# Linux x86_64
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-x86_64-unknown-linux-gnu.tar.gz
tar xzf cc-proxy-x86_64-unknown-linux-gnu.tar.gz
sudo mv cc-proxy /usr/local/bin/

# Linux arm64
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-aarch64-unknown-linux-gnu.tar.gz
tar xzf cc-proxy-aarch64-unknown-linux-gnu.tar.gz
sudo mv cc-proxy /usr/local/bin/

# macOS arm64（Apple Silicon）
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-aarch64-apple-darwin.tar.gz
tar xzf cc-proxy-aarch64-apple-darwin.tar.gz
sudo mv cc-proxy /usr/local/bin/

# macOS x86_64（Intel）
curl -LO https://github.com/Lorem3/cc-proxy/releases/latest/download/cc-proxy-x86_64-apple-darwin.tar.gz
tar xzf cc-proxy-x86_64-apple-darwin.tar.gz
sudo mv cc-proxy /usr/local/bin/
```

Windows 用户可从 Releases 页面下载 `cc-proxy-x86_64-pc-windows-msvc.zip`，解压后将 `cc-proxy.exe` 放入 `PATH` 目录即可。

#### 先决条件

  * **Rust**：需要已安装 `cargo`。

#### 源码构建

```bash
# 克隆仓库
git clone https://github.com/yourusername/cc-proxy.git
cd cc-proxy

# 构建发布版本
cargo build --release

# 全局安装
sudo cp target/release/cc-proxy /usr/local/bin/
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
cc-proxy start

# 查看连接状态与当前路由
cc-proxy status

# 停止代理并恢复 CLI 配置
cc-proxy stop
```

默认会监听 `0.0.0.0:18100` 并自动检测本机可访问的 IP。
将自动提示的地址（如 `http://192.168.1.252:18100`）分享给其他主机，即可让它们共用同一个代理与 model_mapping 配置。

### 机器 B（远程 CLI）示例

当 **机器 A** 执行 `cc-proxy start` 并输出 `Share this URL: http://192.168.0.10:18100` 时，
**机器 B** 可以直接将各 CLI 指向该地址，无需再额外运行代理进程。
在机器 B 上创建以下最小配置文件（记得将 IP 替换为机器 A 实际输出的地址）：

**`~/.claude/settings.json`**

```json
{
  "env": {
    "ANTHROPIC_AUTH_TOKEN": "cc-proxy",
    "ANTHROPIC_BASE_URL": "http://192.168.0.10:18100"
  }
}
```

**`~/.codex/config.toml`**

```toml
preferred_auth_method = "apikey"
model = "gpt-5-codex"
model_provider = "cc-proxy"

[model_providers.cc-proxy]
name = "cc-proxy"
base_url = "http://192.168.0.10:18100"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
```

**`~/.codex/auth.json`**

```json
{
  "OPENAI_API_KEY": "cc-proxy"
}
```

#### 配置

在 `~/.cc-proxy/provider.json` 创建配置文件。

##### 按模型路由（model_mapping）

使用 `model_mapping` 将不同 model 路由到各自的上游。当请求的 `model` 字段包含某个 key（大小写不敏感子串匹配）时，代理将转发到该 key 对应的 `apiUrl`，并注入对应的 `apiKey`。更长（更具体）的 key 优先匹配。

可选配置 `name`，在转发前将请求体中的 `model` 整字段替换为指定值。

```json
{
  "model_mapping": {
    "deepseek-v3": {
      "apiUrl": "https://api.deepseek.com/v1",
      "apiKey": "sk-ds-key",
      "name": "deepseek-v4-pro"
    },
    "mimo-v2.5-pro": { "apiUrl": "https://api.xiaomimimo.com/anthropic", "apiKey": "sk-mimo-key" },
    "sonnet":        { "apiUrl": "https://api.anthropic.com",            "apiKey": "sk-ant-sonnet-key" }
  }
}
```

**匹配规则：**

- 大小写不敏感子串匹配：`"sonnet"` 可命中 `claude-sonnet-4-5`、`claude-sonnet-3-7` 等。
- 更长的 key 优先：`"mimo-v2.5-pro"` 早于 `"mimo-v2.5"` 匹配。
- 配置了 `name` 时，转发前将请求体 `model` 整字段替换为 `name`。
- 若无匹配 key，请求返回错误。

-----

## License

MIT
