<p align="center">
  <img src="assets/logo.png" alt="Captain" width="280">
</p>

<h1 align="center">Captain</h1>

<p align="center"><b>具备生产级纪律的自托管 Agent OS。</b></p>

<p align="center">
  <a href="https://captainagent.fr/"><b>captainagent.fr</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Built%20in-Rust-B7410E?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-green?style=for-the-badge" alt="License">
  <img src="https://img.shields.io/badge/Platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows%20%C2%B7%20Docker-blue?style=for-the-badge" alt="Platforms">
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.fr.md">Français</a> ·
  <a href="README.es.md">Español</a> ·
  <b>中文</b>
</p>

**在你自己的硬件上运行的持久化 AI 操作员。** Captain 是一个 Rust
守护进程，可在会话和重启之间保留对话、项目、记忆、计划任务和智能体状态。它能够执行真实工具、委派给隔离的智能体、通过安全 API
暴露单个智能体，并在后台工作期间保持可观测。审批、预算、循环防护、checkpoint
和审计日志共同约束这种自主能力。你可以在 macOS、Linux、Windows、VPS 或 Docker
上运行 Captain，并通过终端、经过身份验证的 Control 网页应用、Telegram 或 Discord 使用它。

> **公开 Alpha：** Captain 仍在积极开发中，预发布版本之间可能包含 bug、未打磨细节和
> 不兼容变更。请保留备份、审查每项已授予的能力，并且不要将此 Alpha 用于关键工作负载。

<table>
<tr><td width="220"><b>一个二进制文件，一个守护进程</b></td><td>一个编译好的 Rust 核心负责编排智能体、工具、记忆、频道、计划任务和审批。数秒内启动，空闲时资源占用低，以原生服务（launchd/systemd）形式挺过重启，并能自我更新——在聊天中让它更新，批准即可完成。</td></tr>
<tr><td><b>持久化工作</b></td><td>项目、目标、checkpoint、workflow，以及前台或分离式 Live Run 都会持久化。较长的工具输出保留在模型上下文之外，作为有容量限制、已脱敏并通过校验和验证的证据，Captain 可在之后读取、搜索或追踪。重启后，未完成的工作会以 <code>interrupted</code> 状态供检查，而不是消失或被盲目重放。重试必须显式发起、绑定输入摘要并进入审计；对于结果不确定的中断副作用，必须先确认风险。经过身份验证的操作员 API 只公开元数据和有界脱敏的输出尾部，并且仅能取消确实持有实时中止句柄的活动 run。Control 可从全局顶栏打开同一私有清单，提供过滤、实时刷新和仅以文本渲染的有界输出尾部；只有当 runtime 确认 run 确实可中止时才显示取消操作。完整 Ratatui 可通过 <code>/runs</code> 查看同一清单，拒绝过期输出尾部，并要求两步确认取消；独立聊天与 Web 终端仅显示有界元数据。</td></tr>
<tr><td><b>交付前核验证据</b></td><td>纯对话和只读请求仍会实时返回。产生副作用的工作只在有意义的里程碑和交付前检查有序、脱敏的 receipt。文件或配置变更必须有更新且相关的后置条件；仍在运行的 job 和失败检查会被明确标记。Captain 最多进行两轮定向修正，之后会诚实返回未完成状态，而不会声称成功。核验状态可在异常断电后恢复，不保存隐藏推理，也不会盲目重放不确定的副作用。</td></tr>
<tr><td><b>基于证据的调研</b></td><td>批量调研会并行搜索和抓取彼此独立的来源，同时让依赖前序结果的核验保持有序。搜索摘要仅用于发现；只有页面成功读取并记录最终 URL、时间与内容 SHA-256 后，来源才可引用。对于重要结论，Captain 会重新抓取被引用页面，拒绝虚构链接或页面中不存在的原文，统计已声明的出处覆盖率，并从通过审计的 URL 生成 Sources 区块。该审计只证明页面已获取且原文存在，不会凭声明把语义推导当成事实。</td></tr>
<tr><td><b>真实执行，受控管理</b></td><td>Shell、文件、SSH、浏览器、网络调研、代码、文档和媒体。敏感调用需要审批，关键 shell 模式会被拦截，预算限制 token、成本和调用频率。Captain 会将自身持久化的滚动限制与供应商管理的订阅窗口分开显示；对于 Codex，使用百分比和重置时间来自官方实时账户与响应信号，而不是复制的静态配额表。紧凑聊天状态栏只为供应商通用窗口和与当前模型匹配的限制显示独立进度条；其他模型专属配额会标注为不属于当前模型，而 Status 和 Budget 保留完整明细。Ratatui、Web Control 和保留的桌面兼容封装器共享这一契约。相互独立的只读工具可以并行执行，而有依赖或副作用的工作保持有序。</td></tr>
<tr><td><b>人类可读的原生能力</b></td><td>将审核过的 <code>*.captain</code> 文件放入全局或项目 <code>.captain/</code> 目录，Captain 会热加载为类型化的 <code>cap_*</code> 工具。Captain Forge 由内核统一控制依赖、权限、审批、持久化 DAG 执行、崩溃恢复、修订历史、回滚和精确的操作员决策。</td></tr>
<tr><td><b>跟随对话的记忆</b></td><td>会话召回、持久化用户事实、项目状态、知识图谱以及可选的本地 ONNX embedding 提供有边界的上下文，而不会在每一轮重新注入全部历史。已接受的事实会先写入本地持久化连续性日志，在 MemPalace 不可用期间仍可召回，并通过有界退避自动重新同步。</td></tr>
<tr><td><b>一个 Captain，覆盖所有设备</b></td><td>运行一个完整 Hub，并将轻量终端、TUI 或 Desktop Client 配对到同一组会话、项目、记忆、模型和审批。可选执行 Node 只通过出站 HTTPS 443 操作本地工作区，支持代理与企业 CA、显式授权、本地守卫、持久确认，并且断线后不会盲目重放。</td></tr>
<tr><td><b>任意模型，无供应商锁定</b></td><td>Codex（使用你的 ChatGPT 订阅）、Anthropic、OpenAI、Mistral、Groq、Gemini、OpenRouter，以及通过 Ollama 使用的本地模型。Captain 根据实际配置发现模型目录和凭证，不依赖固定数量；上下文预算会跟随所选模型的实时窗口。每个代理都可以单独控制推理级别：Auto 保留模型默认值；当 Codex 公布 Ultra 时，Captain 会使用最大模型推理强度，并且只在根代理上启用有界的主动委派。对于 Codex，Captain 每小时刷新一次目录，并在 Control 以及已配置的 Telegram 中提示新模型；只有在你明确确认并选择会话策略后才会切换。</td></tr>
<tr><td><b>面向真实工作的邮件</b></td><td>通过 OAuth 连接多个 Gmail 账户，或通过 IMAP/SMTP 连接多个与供应商无关的邮箱。可搜索、读取、起草、发送、标记、保存附件，并把确定性匹配路由到指定智能体。凭据不会进入公开配置；已接受任务、自动化游标和崩溃后的不确定结果都可持久化并检查。</td></tr>
<tr><td><b>六个操作中心</b></td><td>Chat, Projects, Automation, Learning, Capabilities 和 Status 是 TUI 与 Control 共用的主界面。Automation 集中管理 Workflows、Triggers、Crons、审批和 Webhooks。</td></tr>
<tr><td><b>智能体即服务</b></td><td>每个智能体都可以接收经过身份验证的外部 ingress，并发送带签名的 HTTP callback。Captain 会自动准备 ingress，并明确指出启用 egress 仍需提供的外部 callback URL。</td></tr>
<tr><td><b>像真正的软件一样可运维</b></td><td><code>captain doctor</code> 会说明哪里出了问题以及如何修复。支持快照与恢复出厂设置（始终先备份）。哈希链式审计日志。健康检查端点。安装向导最终会以一个真正运行、已验证的守护进程收尾——而不是一堆待办事项。</td></tr>
</table>

---

## 快速安装

当前公开早期访问版本：
[v0.1.0-alpha.15](https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.15)。
不可变 Docker 镜像：`ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.15`；
滚动 Alpha 通道：`ghcr.io/vivien83/captain-agent-os:alpha`。

### macOS / Linux / VPS

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.15 bash
```

官方仓库、Release 资产、校验和与容器镜像均为公开内容，无需 GitHub token 或容器仓库登录。

安装脚本会为你的平台下载预编译、经过校验和验证的安装包（无需编译，无需工具链），端到端验证
CLI，并运行一个引导式配置流程，**最终会让 Captain
以后台服务形式真正运行起来**。

同一安装流程会在 daemon 启动前配置由 Captain 管理的内存运行时：uv 0.11.28、隔离的
CPython 3.13.14、MemPalace 3.5.0，以及通过校验和约束的冻结依赖锁。无需系统
Python、手动执行 `pip install` 或提供第二个 API key。`captain memory doctor`
会验证真实的语义读取；启动时若运行时缺失、损坏或权限不安全，Captain 会先修复。若修复失败，Captain 不会在缺少语义内存时声称已达到生产就绪状态。

Captain Full、Captain Console 与 Captain Node 均提供 macOS/Linux 的
`aarch64`、`x86_64` 版本，以及 `x86_64-pc-windows-msvc` 版本。Release 共包含
15 个归档、15 个 SHA-256 sidecar、15 个组件/平台清单、6 个安装脚本、1 个聚合清单
以及受校验和约束的 provenance。

### 轻量 Console 或执行 Node

只安装多 Captain Console，不安装本地 provider、memory、channel、agent loop 或 Full daemon：

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install-edition.sh \
  | CAPTAIN_EDITION=console CAPTAIN_VERSION=v0.1.0-alpha.15 bash
captain-console pair --hub https://your-captain.example
captain-console tui
```

可选的出站 workspace Node 需要单独安装：

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install-edition.sh \
  | CAPTAIN_EDITION=node CAPTAIN_VERSION=v0.1.0-alpha.15 bash
captain-node pair --hub https://your-captain.example --workspace "$PWD"
captain-node service install
```

两个版本都会验证精确 SHA-256；若安装后的版本检查失败，会恢复旧二进制。Node 默认只读，
任何写入都需要 Hub 单独授权。

> **Alpha 签名说明：** Release 归档和校验和会公开，但 macOS 二进制仅使用 ad-hoc
> 签名，尚未经过 Apple notarization。Windows CLI 尚未使用 Authenticode
> 签名。请核验 SHA-256 文件，并预期操作系统在首次启动时要求明确批准。

### 无界面 VPS（完全非交互式）

```bash
export ANTHROPIC_API_KEY=...       # 或任意受支持的提供商 API key
export TELEGRAM_BOT_TOKEN=...      # 可选——见下文
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.15 CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=agent.example.com CAPTAIN_YES=1 bash
```

`vps` 配置模式会安装 systemd 服务、启动服务并验证健康状态。如果检测到 Telegram token，Captain
会向 Telegram API 验证该 token，从机器人的待处理消息中识别出你的聊天，并**向你发送一条确认消息——你与智能体的第一次接触，会在安装完成几秒后出现在你的手机上。**

Captain 可以端到端管理一个公网 HTTPS 域名。请先创建 DNS 记录，然后像
上例一样设置 `CAPTAIN_DOMAIN`，或在交互式安装器中填写。Captain 会把 API
保持在 loopback，事务式配置 Caddy，并且仅在 TLS、Control、公开健康检查和
版本一致性全部通过后才报告完成。

### 无界面 VPS，使用你的 ChatGPT 订阅（Codex，无需 API key）

Codex 是 Captain 内置的默认提供商——不需要 `ANTHROPIC_API_KEY`
之类的东西，只需要你的 ChatGPT Plus/Pro/Pro+ 登录。`CAPTAIN_START=0`
会安装好一切（二进制文件、systemd 服务），但先不启动守护进程，这样下面的就绪检查就不会在你登录之前抢先运行：

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.15/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.15 CAPTAIN_PROFILE=vps \
    CAPTAIN_DOMAIN=agent.example.com CAPTAIN_YES=1 CAPTAIN_START=0 bash

captain login codex        # 会显示一个 URL + 代码——在手机上打开即可，无需本地浏览器
systemctl start captain    # 非 root 安装：systemctl --user start captain
```

### Docker

公开 Alpha 在 GitHub Container Registry 提供 `linux/amd64` 和 `linux/arm64`
镜像，拉取时无需身份验证：

```bash
docker run -d --name captain --restart unless-stopped \
  -p 50051:50051 \
  -v captain-data:/root/.captain \
  -e CAPTAIN_LISTEN=0.0.0.0:50051 \
  ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.15
```

首次启动会生成守护进程 API key，并将其与全部状态一起持久化到命名卷中，该卷可在镜像更新后继续保留。本地
embedding runtime 与受 Captain 管理的 MemPalace runtime 均已在镜像中预置。entrypoint
会在每次启动时运行实时语义 doctor，并在 daemon 启动前修复缺失、损坏或权限不安全的 runtime；即使 bind mount
遮蔽了镜像中预置的状态，也不会静默降级。

公开的 Compose 文件只挂载 Captain 的命名状态卷，不会暴露宿主机文件系统、Docker
socket、PID namespace 或特权模式。运行不可变镜像：

```bash
git clone https://github.com/Vivien83/captain.git && cd captain
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.15 docker compose pull
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.15 docker compose up -d
```

首次启动后再配置所选模型提供商。任何宿主机访问都必须是经过本地审查的显式部署变更；旧的广泛访问
overlay 不属于公开 release 合同。

---

## 快速上手

```bash
captain setup       # 引导式向导：提供商 → 偏好设置 → 频道 → Captain 运行起来
captain             # 完整终端界面
captain chat        # 快速终端聊天
captain doctor      # 诊断任何问题，并给出修复方案
captain update      # 自我更新（或者直接让 Captain 自己更新）
captain status      # 守护进程、智能体、频道、预算、磁盘、健康状态
```

推荐的入门提供商：

- **Codex** — `captain auth login codex`。使用你的 ChatGPT 订阅；无需管理
  API key。
- **Claude** — 在配置前导出 `ANTHROPIC_API_KEY`。

首次对话会触发一次包含七个问题的入门访谈（姓名、语言、时区、回答风格、语音、主动通知和隐私）
——只需一次，覆盖所有界面，并被持久化存储。对于答案范围明确的问题，TUI、Control Web/Desktop
和 Telegram 会提供可点击选项，同时始终保留自由文本回答。

经过身份验证的 Control 网页应用默认位于 `http://127.0.0.1:50051/`。它的六个操作中心与
TUI 保持一致，因此项目、自动化、能力和运行状态不会在不同界面中改变位置。专家终端仍位于
`http://127.0.0.1:50051/terminal`。

---

## 命令行 vs 即时通讯

只需运行一次守护进程；之后可以在任何地方与它对话。所有频道均**默认拒绝**：每个适配器在响应任何人之前，都需要明确的用户白名单。

| 操作 | 终端 | Telegram / Discord |
|---|---|---|
| 与 Captain 对话 | `captain chat` 或 TUI | 给机器人发消息 |
| 审批敏感操作 | TUI 审批面板 | 内联按钮 |
| 中断当前任务 | `Esc` / `Ctrl+C` | `/stop` |
| 守护进程状态 / 重启 | `captain status` / `captain service restart` | 聊天中输入 `status` / `restart` |
| 语音 | `captain voice`（本地 Whisper STT + Kokoro TTS） | 发送语音消息 |
| 更新 Captain | `captain update` | 说"更新自己" → 审批 → 完成 |

Captain 会在启动后检查官方发布频道，之后每 12 小时检查一次。配置了精确的
Telegram 操作员聊天和显式用户白名单后，Rich 卡片会提供**立即更新**、
**24 小时后提醒**和**拒绝此版本**。该决定由控制平面直接处理，不会触发模型
轮次；没有点击就不会安装。宿主机更新会验证 release 的 SHA-256，Docker 和
不支持自更新的平台仍由操作员管理并接受后续复查。持久化监控状态可在
`captain status` 和 `GET /api/status` 中查看。

配置公网域名后，Captain 还会每五分钟检查本地/公网端口与健康状态、DNS、TLS、
反向代理路由以及精确版本一致性。`captain doctor --full`、Status API 和 Control
读取同一份可抵御意外停机的快照，并给出明确修复操作，而不会暴露原始网络错误。

---

## 你可以让它做什么

```text
检查我的 VPS：磁盘、内存、失败的服务——修复可以安全修复的部分。
在网上调研 X，并生成一份带来源引用的 PDF 报告。
监控这个文件夹，并通过 Telegram 向我总结新增的文档。
每天早上 8 点：我的日历、天气、日志中任何异常情况。
通过 SSH 连接备份服务器，验证昨晚的任务确实执行成功了。
更新你自己。
```

底层会对内置工具进行语义选择，只有相关 schema 才会传给模型。Captain 还支持受治理的 skill、MCP
工具服务器、多智能体委派、workflow、浏览器自动化，以及可由智能体重新查看、取消或按依赖排序的持久化工具运行。

---

## 文档

| 指南 | 内容 |
|---|---|
| [Getting Started](docs/getting-started.md) | 安装 → 配置 → 第一次对话 |
| [Configuration](docs/configuration.md) | `config.toml`、提供商、模型，所有选项 |
| [CLI Reference](docs/cli-reference.md) | 所有命令与参数 |
| [Providers](docs/providers.md) | 模型提供商、身份验证、配置模型优先与显式回退 |
| [Channel Adapters](docs/channel-adapters.md) | Telegram、Discord、Signal、邮件配置 |
| [安全](docs/security.md) | 身份验证、能力控制、密钥、审批与审计记录 |
| [Built-in Tools](docs/captain-tools/) | 按类别划分的工具文档 |
| [Architecture](docs/architecture.md) | Crate 结构、智能体循环、内核设计 |
| [API Reference](docs/api-reference.md) | REST 端点、身份验证、流式传输 |
| [Hub、Client 与 Node](docs/hub-clients-nodes.md) | 一个中央 Captain、轻量界面与出站本地执行 |
| [VPS Deployment](docs/deployment/github-vps-install.md) | 无界面安装、反向代理、HTTPS |
| [MCP](docs/captain-tools/mcp.md) | 外部工具服务器与传输协议 |
| [Troubleshooting](docs/troubleshooting.md) | 常见问题及其解决方法 |
| [0.1.0-alpha.15 Release Notes](docs/releases/v0.1.0-alpha.15.md) | 多 Captain Console、原生 Node 服务与独立轻量 bundle |
| [0.1.0-alpha.14 Release Notes](docs/releases/v0.1.0-alpha.14.md) | 一个 Hub、轻量 Client、出站 Node 与崩溃安全的分布式工作 |
| [0.1.0-alpha.13 Release Notes](docs/releases/v0.1.0-alpha.13.md) | 自适应交付核验、崩溃安全证据与强化的主机更新 |
| [0.1.0-alpha.12 Release Notes](docs/releases/v0.1.0-alpha.12.md) | 持久 Live Runs、基于证据的研究、已验证构件与托管 VPS 域名 |
| [0.1.0-alpha.11 Release Notes](docs/releases/v0.1.0-alpha.11.md) | 原生邮件、持久集成、审计收尾与本地 CI |
| [0.1.0-alpha.10 Release Notes](docs/releases/v0.1.0-alpha.10.md) | 生产级加固、持久运行与可验证的本地发布 |
| [0.1.0-alpha.9 Release Notes](docs/releases/v0.1.0-alpha.9.md) | 持久工作流学习与原生更新监控 |
| [0.1.0-alpha.7 Release Notes](docs/releases/v0.1.0-alpha.7.md) | 已提交状态持久化、受监督重启、真实上下文与 TUI 直接记忆写入 |
| [0.1.0-alpha.6 Release Notes](docs/releases/v0.1.0-alpha.6.md) | Telegram Rich Messages、实时工具面板、临时进度与可靠交互控制 |
| [0.1.0-alpha.5 Release Notes](docs/releases/v0.1.0-alpha.5.md) | 干净关停、记忆隐私、实时模型身份与单代理首次启动 |
| [0.1.0-alpha.4 Release Notes](docs/releases/v0.1.0-alpha.4.md) | 权威更正、完整活动记忆检索与 CLI 连续性 |
| [Docs Status (DOC2)](docs/DOCS_STATUS.md) | 当前契约、冻结界面和历史文档 |

> `docs/` 目录下的详细指南目前仅提供英文版本。

---

## 安全态势

- API 默认绑定在 `127.0.0.1`，若在公网接口上启动且未配置身份验证，会**拒绝启动**。
- 访问网页/API 需要登录会话或 bearer API key；网页配置编辑器需要身份验证。
- 敏感工具会经过审批流程；极高风险的 shell 模式会被拦截，或无论策略如何都强制要求一次性审批。
- 按智能体设置预算：token、按小时/天/月计算的成本、工具调用频率。
- 循环检测器：针对重复调用、来回摆动模式，以及连续失败的熔断机制。
- 频道白名单默认拒绝；哈希链式审计日志；密钥保存在 `secrets.env`、
  加密保险库，或通过 `secret-sources.toml` 声明的权威只读外部文件中，
  绝不出现在配置文件里。

状态数据保存在 `~/.captain/` 下——`config.toml`
是唯一的可信数据源，变更后会热重载。

---

## 开发

```bash
cargo test --workspace              # 完整测试套件
cargo build --release -p captain-cli
scripts/release-readiness.sh         # 完整本地 release gate
CAPTAIN_VERSION=vX.Y.Z scripts/release-all.sh  # 本地顺序构建全部 15 个 bundle
CAPTAIN_VERSION=vX.Y.Z scripts/publish-release-local.sh
docker build --build-arg CAPTAIN_BUILD_VERSION=vX.Y.Z -t captain:vX.Y.Z .
```

`release-all.sh` 会严格逐一构建 Full、Console 与 Node 的两个 macOS、两个 Linux
以及一个 Windows 目标；Windows 交叉构建使用 `cargo-xwin`、LLVM 和 NASM。完整
release gate 通过且工作树干净后，`publish-release-local.sh` 会验证 52 个主机
asset，再加入一份 SLSA v1 provenance 声明及其校验和（共上传 54 个 asset），推送当前分支，
依次构建并推送 `linux/amd64` 和 `linux/arm64`，最后才组装 GHCR 索引并发布 tag
与 GitHub Release。镜像会直接复用两个已验证的 Linux release 二进制文件，而不是
在模拟环境中重新编译 Captain。组装镜像前，发布脚本会从维护者本机的 Captain
缓存中准备一个由校验和固定的 FastEmbed snapshot，并放入 Git 忽略的
`dist/docker/`。该缓存既不会提交到仓库，也不会加入 54 个 release asset；
Docker 构建还会再次验证它，因此无需依赖实时可用的模型 CDN。具体契约及当前签名
限制见 [Release Provenance](docs/release-provenance.md)。只需运行一次
`gh auth refresh -h github.com -s read:packages,write:packages`
完成认证；不要在
命令行中传递 token。GitHub release workflow 仅作为显式手动 fallback，推送 tag
不会自动触发它。CI 仍可通过显式手动触发用于格式检查、严格 Clippy、安全与
secret 审计以及 workspace checks/tests。

---

## 许可证

采用 [MIT](LICENSE-MIT) 或 [Apache 2.0](LICENSE-APACHE) 双重许可证，可自行选择。
