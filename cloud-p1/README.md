# Cloud P1｜授权模型 API 网关（v0.1.22 基线）

**状态：本地端到端模拟测试通过，尚未部署云端，尚未接入真实收费或免费模型。**

P1 是与 Windows Tauri 桌面客户端、WorkBuddy 账号池完全解耦的独立 Rust HTTP 服务。它只转发到**运行者已获授权的、符合 OpenAI Chat Completions 格式的模型 API**。不读取 WorkBuddy 登录状态、不调用 WorkBuddy 私有接口、不管理任何个人客户端令牌，也不会执行 AI 返回的工具调用。

## 架构

安卓 DSH Go / OpenAI 兼容客户端 → HTTPS 受保护的外层网关（尚未实现） → P1 回环服务 → 已获授权的模型 API

如果只是想在国内安卓上使用百炼模型，**直接把 DSH Go 连接到百炼官方 HTTPS API 更简单，不需要 P1，不需要独立服务器或备案**。P1 适合之后需要统一接入、模型白名单、安全代理时使用。

## 功能

- GET /healthz：本地健康检查（不包含 API Key）
- GET /v1/models：需要客户端 Bearer Key，返回配置好的单个模型
- POST /v1/chat/completions：需要客户端 Bearer Key；支持 stream=true 的 SSE 和非流式 JSON
- 合规模型 Provider 独立配置：客户端访问密钥与上游 API 密钥分离
- 原样透传上游模型的正常回答和工具调用结构；**不执行任何工具**
- 模型白名单：只能请求 P1_MODEL 配置的模型
- 默认最大请求体 1 MiB，最多 8 个并行请求，上游网络超时 180 秒
- 服务器默认只绑定 127.0.0.1:18791，拒绝非回环地址
- 生产上游只允许 HTTPS 域名的标准 443 端口，禁用 HTTP 跳转；测试需要显式开启本地 HTTP 回环例外
- 上游错误不透传隐私或内部错误内容
- /v1/responses 尚未实现

## 运行示例（阿里云百炼，北京地域）

以下仅是配置示例，真实 API Key **仅保存在你自己设备的环境变量或安全配置中，不要上传 GitHub 或发送到聊天中**。

PowerShell:

    $env:P1_CLIENT_API_KEY = 'your-own-unique-24-plus-character-client-key'
    $env:P1_UPSTREAM_API_KEY = 'your-own-authorized-bailian-key'
    $env:P1_UPSTREAM_BASE_URL = 'https://dashscope.aliyuncs.com/compatible-mode/v1'
    $env:P1_MODEL = 'qwen-plus'
    cargo run --manifest-path cloud-p1/Cargo.toml

从同一台电脑发送：

    curl.exe -H "Authorization: Bearer YOUR_LOCAL_CLIENT_KEY" http://127.0.0.1:18791/v1/models

官方下载文档（北京地域）：https://help.aliyun.com/zh/model-studio/base-url

**重要：**按量计费与业务空间专属的 API Key/地址应配套使用；使用前确认账户的免费额度、到期策略和停止计费选项。百炼 Token Plan / Coding Plan 有专用使用限制，不得把面向开发工具交互的套餐 Key 视为通用后端模型 API Key。

## 安全隔离测试

从 GitHub 仓库根目录：

    cargo test --locked --manifest-path cloud-p1/Cargo.toml
    cargo build --locked --manifest-path cloud-p1/Cargo.toml
    python cloud-p1/tests/contract.py cloud-p1/target/debug/qiehuan-cloud-p1.exe

Linux 去掉最后一行的 .exe。测试启动本机 HTTP 假上游，完全不访问阿里云或 WorkBuddy；自动选择空闲的本地端口。

截至 2026-11-11，Windows 11 + Rust 1.96.1：Rust 单元测试 **2/2**，实际 HTTP 合约测试 **9/9**，编译成功。Linux CI 另行验证，未通过时不能声称 Linux 已验收。

## 上线之前必须补足

1. 已授权 API Key 的真实端到端调用，实际收费/免费额度和过期后的停用策略。
2. 安全的 HTTPS 入口、部署环境 Secret 管理、日志脱敏、轮换与限流；不要把 P1 直接暴露公网。
3. Linux 平台编译、容器运行、运维监测与安卓 DSH Go 的实际连接测试。
4. 根据中国大陆节点、对外提供服务方式，核实 ICP 备案/托管平台规则和几个月的零预算要求。
5. 真实 WorkBuddy 多账号池的客户端脱离认证尚未解决，不能把 P1 视为其云端迁移成功。

## DSH Go 测试参数（只有在 HTTPS 外层网关上线后）

- 协议：OpenAI Chat Completions
- Base URL：经授权的 HTTPS 网关地址，末尾为 /v1
- Key：P1_CLIENT_API_KEY，非上游模型 Key
- Model：P1_MODEL 指定的模型名（如 qwen-plus）
- 不要选择仅支持 /v1/responses 的渠道；P1 目前没有实现该接口

如果用户只需要几个月测试，优先使用官方模型 API 直连来减少云服务成本；不要为此先购买云服务器。
