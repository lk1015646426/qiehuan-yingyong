# Cloud P0｜独立 API 合约原型

基线：切换应用 v0.1.22（2026-11-11）。

**这是隔离测试服务，不是 WorkBuddy 云端账号池。** 不读取真实账号、不刷新真实凭证、不调用腾讯模型接口、不消耗账号积分。

## 对 v0.1.22 的代码审计

与 v0.1.17 公开 Git 基线比较，以下 WorkBuddy 后端没有改变：

- src-tauri/src/modules/workbuddy_pool.rs
- src-tauri/src/modules/workbuddy_account.rs
- src-tauri/src/modules/workbuddy_auth_crypto.rs
- src-tauri/src/modules/secure_account_storage.rs

主要阻塞项：

1. 桌面耦合：账号池通过 Tauri 启动，修改 ~/.workbuddy/models.json，监听本机地址 127.0.0.1:18789。
2. 身份认证：账号导入和部分官方加密字段读取依赖本地 WorkBuddy 认证快照及 Electron 客户端。源码包含网络刷新路径，但尚未在 Linux、无客户端、长期运行的条件下完成验证。
3. 生产协议：现有账号池只支持 GET /v1/models 和流式 POST /v1/chat/completions，不支持 /v1/responses。云端服务还需要 HTTPS、限流、日志脱敏、秘密管理、并发与超时验收。

## P0 原型功能

- 独立 Rust Cargo crate；无 Tauri 依赖，适用于标准 Rust Windows/Linux 工具链。
- 默认只监听 127.0.0.1:18790，并拒绝非回环绑定。
- 必须设置 P0_API_KEY（长度 >= 16 的测试字符串）；不可使用真实账号 Key。
- GET /healthz：返回 upstream=disabled；无任何外部 AI 请求。
- GET /v1/models：需要 Bearer，返回 workbuddy-pool-p0-mock。
- POST /v1/chat/completions：需要 Bearer；支持 stream:true 的多帧 SSE 和非流式 JSON。
- 固定回答 P0_MOCK_OK，不具备真实 AI 推理能力。
- POST /v1/responses 暂不支持，返回 HTTP 404。
- 不使用真实 WorkBuddy 账号或凭证。

## 测试命令

从仓库根目录运行：

    cargo test --manifest-path cloud-p0/Cargo.toml
    cargo build --manifest-path cloud-p0/Cargo.toml
    python cloud-p0/tests/contract.py cloud-p0/target/debug/qiehuan-cloud-p0.exe

Linux 将 exe 后缀去掉。已缓存 Rust 依赖时可使用 --offline。

手工运行（Windows PowerShell）：

    $env:P0_API_KEY = 'local-fixture-key-not-real-secret'
    cloud-p0\target\debug\qiehuan-cloud-p0.exe

不要将密钥上传 GitHub；此原型禁止开放公网端口。

## P1 云端化任务

1. 优先接入具备正式授权的模型 Provider，不将个人客户端未公开接口当作第三方公共模型 API。
2. 将 HTTP 接口、模型选择、鉴权与 Provider 拆分。云端秘密应存储在 Secret Manager，且支持轮换。
3. 实现流式与非流式请求、模型列表、错误映射、工具调用与合理并发；DSH Go 需要 Responses API 时额外适配。
4. 生产网关必须提供 TLS、限流、安全日志、请求大小限制、监测与恢复措施。
5. 取得合法授权与无客户端端到端验收后，再评估中国大陆部署、备案要求与几个月免费额度是否足够。

当前成果仅证实“脱离 Tauri 的 OpenAI 兼容接口可以独立运行”，不是 WorkBuddy 账号池已经迁移成功。


## 2026-11-11 实测结果

- Windows 11 + Rust 1.96.1：cargo test --offline，4/4 单元测试通过。
- cargo build --offline：构建成功。
- Python 标准库运行真实 HTTP 合约测试：7/7 通过；验证鉴权、模型列表、JSON 对话、SSE、多项错误码。
- 测试使用临时模拟 Key 与自动选择的 127.0.0.1 本机端口，不读取真实账号文件，不请求上游模型。
- Linux 构建和安卓 DSH Go 端到端尚未验证；P0 原型并不处理真实 WorkBuddy 凭证。
