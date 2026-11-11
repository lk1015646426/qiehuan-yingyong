# 决策记录：个人开发者无法进入 WorkBuddy 官方云端任务通道

日期：2026-11-11
状态：**BLOCKED：个人主体无法在当前后台创建“硬件接入”应用**

## 用户实测

用户使用个人主体登录 WorkBuddy 开放平台后，发现“硬件接入”要求企业认证，个人认证无法继续。这是实际控制台反馈，应优先于此前的技术推断。

## 官方证据与可申请范围

- [第三方应用官方文档](https://open.workbuddy.cn/docs/third-party-app)：云端任务权限 user.task.invokable / user.task.readable 在“硬件接入”等经审核的应用中；积分兑换应用只能申请部分积分权益相关 Scope。
- [Open API 文档](https://open.workbuddy.cn/docs/openapi)：创建云端任务与 ACP 连接均以前述已获批第三方应用/OAuth 授权为前提。
- [开发者入驻说明](https://open.workbuddy.cn/docs/onboarding)：个人认证不同于企业认证，认证本身不赋予全部应用类别。
- 用户个人后台已确认：硬件接入应用不能在个人身份下继续创建。

## 现在的工程决策

1. **暂停官方 OAuth/ACP 真实接入**；保留 PR #2 作为已验证的协议研究，不合并 master，不继续开发需要获批 Scope 才能验收的生产功能。
2. **不试图使用虚假企业信息、借用公司身份或绕过审核**。
3. 回到用户原始需求：已运行的本地 WorkBuddy 多账号池，目标是电脑关机后仍有合法授权的模型 API。
4. 对原本的 `src-tauri/src/modules/workbuddy_pool.rs`、`workbuddy_account.rs`、`workbuddy_auth_crypto.rs`、`secure_account_storage.rs` 做**不读取真实凭证**的静态审计：记录 Windows/Electron 依赖、模型请求耦合与未授权上游风险，明确 Linux/容器能否无客户端运行。
5. 如 WorkBuddy 只提供个人桌面会话调用，没有正式服务端模型 API 授权，则停止“个人额度云端化”路线，不应把 Mock 测试成功当作许可。
6. 个人短期零成本替代路线：使用具有明确 API 授权的模型服务直连安卓 DSH Go，不要求自建服务器、ICP备案或本机电脑常开。是否免费以账号额度为准。

## 现实验收条件

- **官方云端任务路径**：拥有合资格企业/合作方主体和相应 Scope，且确认费用后才可继续。
- **原账号池云端路径**：获得允许服务端调用的正式授权；完成无桌面依赖的真实运行、合法托管与安全检查。
- **DSH Go 端到端**：在实际安卓设备与 HTTPS 地址上测试，Mock/CI 不能替代。

结论：**当前堵点不是编程，而是个人开发者应用资格与现有 WorkBuddy 个人积分的授权边界。**
