# WorkBuddy 0.1.22 原账号池｜无桌面客户端运行可行性审计

**状态：NOT READY / 云端部署禁止。** 2026-11-11，针对当前 GitHub 仓库 master（桌面稳定版 v0.1.22）源代码，完全不读取真实账号和凭证。

## 用户的真实目标

- 国内平板 / 安卓 DSH Go，使用较高能力模型；
- 电脑关机仍能工作；
- 不购买云服务器、没有自有ICP备案域名，短期几个月尽可能免费；
- 优先利用自己原有 WorkBuddy 多账号池，但不得混淆“桌面个人会话”与“平台正式授权服务端 API”。

## 已证实的架构事实

| 检查领域 | v0.1.22 代码证据 | 结果 |
|---|---|---|
| 原始模型上游 | workbuddy_pool.rs 中 UPSTREAM 固定为 https://copilot.tencent.com/v2/chat/completions | **不是** WorkBuddy 开放平台所列的 OAuth /openapi/v2 云端任务接口 |
| 模型 API 入口 | 绑定 127.0.0.1:18789， GET /v1/models、POST /v1/chat/completions | 目前只能在运行该应用的电脑本机访问 |
| 模型格式 | 只接受工作池前缀 model ID，要求 stream=true | 不是覆盖全部 OpenAI 接口的通用 API |
| 平台生命周期 | #[tauri::command] + tauri::async_runtime::spawn_blocking | 当前实现与桌面 Tauri 运行时耦合 |
| 本地模型注册 | 修改用户 home 下 .workbuddy/models.json | 强依赖用户本机 WorkBuddy 客户端模型配置 |
| 账号导入 | 默认 LOCALAPPDATA/CodeBuddyExtension/Data/Public/auth/workbuddy-desktop.info | 默认 Windows 客户端来源 |
| 官方字段解密 | workbuddy_auth_crypto.rs 调用 process._linkedBinding('electron_browser_workbuddy_storage') 并启动 WorkBuddy Electron Node helper | 不具备无 WorkBuddy 可执行文件的密钥获取来源 |
| 管理数据加密 | secure_account_storage.rs 的 AES-256-GCM 帐号详情，单独本地密钥文件 | 直接复制加密文件到服务器既不安全，也不能视为合法授权 |
| 认证恢复 | workbuddy_account.rs 通过 /v2/plugin/auth/token/refresh 及 /v2/plugin/account | 依赖客户端相关的非 OpenAPI 端点；Linux 无客户端长期稳定性、服务端许可未证实 |
| 上游请求身份 | workbuddy_pool.rs 中使用用户 token、X-User-Id 等会话标识头 | 桌面会话认证语义，不等于有权作为远程模型服务提供者 |
| 请求处理 | 单一 worker 从 incoming_requests() 同步调用 handle()/relay() | 长流式响应可能阻塞后续请求；生产前需要独立限流和并发管理 |

**可复用的技术部分：** HTTP 模型路由/校验、SSE 流输出、单请求不重放、错误分类、选择账号/冷却状态等逻辑。即使复用，也只能配合**获得官方明确许可的服务端 Provider**；这些逻辑不能自行赋予调用私有会话端点的权利。

### 两种“能在 Linux 运行”不可混为一谈

1. Linux CI 通过编译、模拟 HTTP 测试，说明**本机协议实现**可跨平台。
2. Linux 服务用真实 WorkBuddy 账号稳定、合法地调用模型、管理机密、被安卓公网访问，是另一件事。

目前只做到了第一类的一部分。没有真实授权方式证据前，不会让服务读取或批量迁移个人登录令牌、尝试模拟私人客户端身份，也不会申请公网地址。

## 部署闸门（本项目新增）

source-only 检测器 **workbuddy-headless/audit.py** 仅检查四份既定 .rs 源码文件，不读取 LOCALAPPDATA、账户库、密钥、浏览器缓存或任何个人数据。

- 每条风险规则返回 file、line、rule code、status、repair suggestion；**不输出源代码段或 token**。
- 缺少任意源文件时进入 insufficient_source；不把“没找到某个标记”当成风险已解决，而是要求人工核查。
- 默认输出静态审计信息、JSON 模式可供 CI/后续 Codex 使用。
- 参数 --require-cloud-ready 采用 **fail-closed**：尚无正式服务端模型 API 授权证据、无无客户端实测与安全验收，直接返回退出码 3。
- GitHub Actions Linux CI 验证检测器和此失败即关闭规则，**CI 绿色并不意味着允许云端部署**。

运行命令：

    python3 workbuddy-headless/audit.py --root . --json
    python3 -m unittest discover -s workbuddy-headless/tests -p "test_*.py" -v
    python3 workbuddy-headless/audit.py --root . --require-cloud-ready

最后一条**预期返回退出码 3**，表示部署条件仍然不满足，而不是工具故障。

## 下阶段技术决策

| 路线 | 是否继续 | 原因 |
|---|---|---|
| A. 原样把 WorkBuddy 会话池搬到 Linux 云服务器 | **暂停** | Electron 密钥、客户端快照、私有接口授权/持续刷新、合规托管均未具备 |
| B. 取得 WorkBuddy 正式服务端模型 API 授权再抽取账号池纯逻辑 | **有条件继续** | 必须是官方许可的服务端模型 API，而非个人桌面会话 |
| C. WorkBuddy 开放平台云端任务/ACP | **受阻** | 用户在 2026-11-11 通过开放平台确认“硬件接入”必须企业认证，个人开发者无法创建；见 PR #2 |
| D. 安卓 DSH Go 直连正式模型 API（如深度思考模型） | **短期主线** | 无需本机电脑与自建服务器，费用依具体免费额度而定；已另行开发通用授权 P1 网关，但非必需 |

如果希望两条线均继续：WorkBuddy 原本的桌面多账号池保持原状，作为本地工具使用；长期云端服务只能基于明确授权的模型接口继续。

**本审计结论不是法律意见。**“未证实服务端调用许可”意味着不得自动放行部署，不等于已判定特定行为必然违法。可依据 WorkBuddy 官方文档、条款或支持答复更新结论。

## 官方参考

- WorkBuddy 第三方应用与可申请范围：https://open.workbuddy.cn/docs/third-party-app
- WorkBuddy Open API（OAuth、云端任务、ACP）：https://open.workbuddy.cn/docs/openapi

以上文档仅支持官方开放平台的相应业务 API，并没有说明可将个人客户端模型会话接口作为独立远程模型供应端点。

## 下一次工作不应重复的事

- 不要再次开发只返回静态 P0_MOCK_OK 的“假 WorkBuddy 云端 API”来宣称账号池迁移成功。
- 不要要求用户提供 Token、refresh token 或加密密钥来尝试未经授权的上游。
- 不要动正式 master 或当前 Windows 0.1.22 应用及账号数据。
- 必须先拿到真实的**正式服务端模型接口授权证据和费用条件**，才能从“接口模拟”进入“真正可用”阶段。
