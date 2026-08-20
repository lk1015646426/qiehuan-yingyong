# WorkBuddy 网络链路隔离 Implementation Plan

> **For agentic workers:** Inline execution in this session; each task is verified before the next task.

**Goal:** 隔离 WorkBuddy 国内查询直连链路，并为 GitHub 同步增加直连优先、VPN 代理回退与明确错误提示。

**Architecture:** WorkBuddy 状态查询创建禁用系统代理的专用 reqwest 客户端。GitHub 真实 runner 将一次调用拆为直连尝试和网络错误限定的代理回退，代理来源优先使用现有环境变量，Windows 下补充读取系统代理设置。

**Tech Stack:** Rust、Tauri、reqwest 0.12、GitHub CLI、现有 FakeGitHubRunner 测试抽象。

## Global Constraints

- 不将 WorkBuddy Secret 或 Token 发送到第三方镜像。
- GitHub Secret 只能通过 `gh` stdin 传递。
- WorkBuddy 查询不得继承 Clash/Windows 系统代理。
- 保留现有未提交改动，不回滚无关文件。

### Task 1: WorkBuddy 直连客户端

**Files:**
- Modify: `src-tauri/src/utils/http.rs`
- Modify: `src-tauri/src/modules/workbuddy_status.rs`

- [ ] 添加测试验证直连客户端禁用系统代理。
- [ ] 运行定向测试并确认修改前失败。
- [ ] 增加 `create_direct_client` 并让 WorkBuddy 状态查询使用它。
- [ ] 运行定向测试确认通过。

### Task 2: GitHub 直连优先与代理回退

**Files:**
- Modify: `src-tauri/src/modules/work_cn_github.rs`

- [ ] 添加测试验证网络失败才触发代理回退，认证失败不重复。
- [ ] 运行测试确认修改前失败。
- [ ] 实现直连环境、代理环境和 Windows 系统代理读取。
- [ ] 让真实 runner 在网络错误时进行一次代理重试并返回脱敏提示。
- [ ] 运行 GitHub 定向测试确认通过。

### Task 3: 错误提示与全量验证

**Files:**
- Modify: `src-tauri/src/modules/workbuddy_status.rs`
- Modify: `src-tauri/src/modules/work_cn_github.rs`

- [ ] 区分 HTTP 401、WorkBuddy 网络错误和 GitHub 需要 VPN 的错误。
- [ ] 运行 Rust 测试、前端类型检查和生产构建。
- [ ] 运行 NSIS 打包并检查安装包路径。
