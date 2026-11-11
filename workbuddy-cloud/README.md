# WorkBuddy 云端路线｜官方开放平台独立验证（2026-11-11）

**基线：切换应用 v0.1.22。此目录为独立研究实现，不影响原 Windows 桌面程序、账号池及登录态。**

## 用户目标

- 中国大陆网络，平板上使用 DSH Go，电脑关机仍可调用。
- 仅未来几个月，云端费用尽量为零。
- 优先复用 WorkBuddy 已有能力，但不公开或导出个人客户端身份数据。

## 双路线的关键区别（不可混为一谈）

| 项目 | 原 WorkBuddy 多账号池 | WorkBuddy 官方 Open API |
|---|---|---|
| 服务类型 | 本地 Chat Completions SSE 网关 | OAuth 2.1、云端任务、ACP |
| 账户授权 | 依赖 Windows 客户端会话快照及内部认证细节 | 通过用户同意的第三方应用 OAuth 2.1 |
| 电脑关机 | 原服务无法运行 | 官方云端任务理论上不依赖本机助理 |
| DSH Go 直接兼容 | 本地接口与 OpenAI Chat Completions 部分兼容 | **不能直接兼容**，ACP 是 SSE + JSON-RPC 双通道 |
| 能否复用个人模型额度 | 当前本地方案使用该类额度 | 官方文档**未承诺**能把个人模型积分转成 Chat Completions 接口 |
| 主要障碍 | 合规授权、服务端无客户端认证、并发、加密密钥托管 | 第三方应用注册与权限、ACP 适配、成本、部署 |

原本的 WorkBuddy Pool 核心（Tauri）在 0.1.22 相比 0.1.17 未有变化；不得将旧项目直接复制到无客户端 Linux 云服务器并宣称可用。

## 官方文档确认

- 开发者必须注册第三方应用，获取 clientId / clientSecret；服务端安全保存 secret。
- OAuth 用户同意后得到 access_token 与 refresh_token。
- POST https://www.workbuddy.cn/openapi/v2/tasks，权限 user.task.invokable，可创建云端任务。
- GET /openapi/v2/tasks/{task_id}，权限 user.task.readable，可查任务与 ACP token。
- ACP：GET SSE + POST JSON-RPC，需关联 Acp-Connection-Id。
- 方法：initialize → session/load → session/prompt。
- SSE 通道推送 session/update 与 session/request_permission；后者必须经用户明确审批。
- 一轮对话的完成标志是 session/prompt 对应 id 的 result.stopReason；**不可只看到模型输出即宣布完成**。

权威资料：
- https://open.workbuddy.cn/docs/openapi
- https://open.workbuddy.cn/docs/third-party-app
- https://open.workbuddy.cn/docs/developer-agreement

## 当前代码

- official.py：OAuth 授权 URL、CSRF state 校验；官方任务创建/查询；严格限制调用路径与 HTTPS。
- oauth_tokens.py：授权码交换与 refresh_token 刷新，仅使用官方 /openapi/v2/token；在内存中处理且隐藏对象打印内容，不自动存储。
- acp.py：ACP JSON-RPC 方法构造、分块 UTF-8 SSE 解码、会话事件分类。对 permission_request 不自动同意、执行工具或进行敏感操作。
- tests/：只用**虚构**凭证、模拟任务与模拟 SSE；不接触 WorkBuddy 真实账号，不消耗积分。

## 快速测试（无依赖）

在仓库根目录执行：

    python -m unittest discover -s workbuddy-cloud/tests -p "test_*.py" -v

GitHub Actions 将使用 ubuntu-latest + windows-latest 跑相同测试。

## 为什么暂时没有真实调用？

1. 目前没有获得用户在 WorkBuddy 开放平台创建的第三方应用 ID、回调地址与获批任务权限。
2. 真实 OAuth 授权需要用户确认，不能使用或逆向本地客户端私有 token 来代替。
3. 官方云端任务/ACP 并非 DSH Go 原生的 OpenAI Chat Completions 协议，完整双向流式适配尚未完成。
4. ACP 会话可能包含需要人工确认的任务权限，必须设计移动端交互入口；没有此功能时应拒绝高权限任务，而不是自动通过。
5. 没有确认腾讯 WorkBuddy 官方云端任务调用是否使用与本地账号池相同的模型、积分或费用配额。

## 后续 P1 验收顺序

1. **开发者账号与正式应用能力**：检查创建第三方应用的权限、是否能获取 user.task.invokable / user.task.readable、OAuth callback 要求与实际费用。
2. **获授权的单用户沙盒验证**：官方 OAuth 授权 → 创建一个普通文字任务 → 获取 ACP link/token → SSE 消息与 prompt 结束。严格限定请求数量、无批量账号轮换。
3. **有实测样本后**才开发 ACP 双通道到 OpenAI Chat Completions SSE 的保守桥接。必须保留角色、消息顺序、工具/权限请求及终止标记语义。
4. **目标环境检查**：DSh Go 的 OpenAI Chat Completions 兼容、安卓 HTTPS 连接、中国大陆可达性、合法托管和零预算期限。
5. **最终路线决策**：如果官方任务接口不能提供原模型池额度，就将两者分开，不把云端任务接口误称为多账号池云端化成功。

**严格不做：**使用官方客户端私有加密密钥迁移到 Linux、将桌面 refresh token 导出公开仓库、绕开第三方平台授权、以多账号规避平台限额、将无保护的测试服务器暴露公网。
