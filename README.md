# 切换应用

一款 **Windows 专用**的 [TRAE Work CN](https://www.trae.cn/) 多账号管理工具，支持最多 4 个积分制账号的一键切换与免登录打开。

> 本项目基于开源项目 [Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) `v1.3.16` 改造而来，复用其成熟的 TRAE 账号管理与切号实现。详见 [NOTICE.md](./NOTICE.md)。

当前项目仓库为私有仓库：[lk1015646426/qiehuan-yingyong](https://github.com/lk1015646426/qiehuan-yingyong)。本次版本已完成 Work CN 与 WorkBuddy 多账号切换链路的稳定性修复。

## 功能定位

每个账号首次正常登录一次并导入后，日常切换只需在管理器中点击「切换并打开」：

```text
点击「切换并打开」
→ 管理器保存当前账号最新会话
→ 正常关闭当前 TRAE Work CN
→ 注入目标账号完整登录快照
→ 启动官方 TRAE Work CN
→ 校验打开后的 UID 是目标账号
→ 完成
```

正常情况下无需再次输入密码、扫码、手动退出账号或编辑 `storage.json`。

主指标为 **总积分 / 已用积分 / 剩余积分**；积分签到继续由现有 GitHub Actions 完成，本软件只查询积分、维护登录快照并同步签到凭证。签到工作流由独立的 [daily-checkin](https://github.com/lk1015646426/daily-checkin) 仓库维护，本项目不新增或替代该工作流。

## WorkBuddy 多账号

侧栏中的 **WorkBuddy** 页面与 TRAE Work CN 账号库完全隔离，支持导入任意数量的官方客户端登录快照、加密保存、切换并启动客户端，以及选择参与云端自动签到的账号。

- 首先在官方 WorkBuddy 客户端完成正常登录，再导入当前账号；工具不会自动输入验证码、扫码或绕过登录限制。
- 认证快照只保存在本机并使用 AES-256-GCM 加密。GitHub 仅接收签到需要的 access token，绝不接收完整认证文件、refresh token、session state 或完整手机号。
- 切换会完整原子替换认证文件。客户端未能正常退出时，必须在确认后才会强制结束进程；取消不会修改认证文件。
- 默认自动检测 WorkBuddy；可在“路径设置”中填写 EXE 或认证文件路径。认证文件路径会同时用于导入、切换与后台凭证监测。
- 启用“参与自动签到”的账号会同步到一个 `WORKBUDDY_ACCOUNTS_JSON` GitHub Secret。旧的 `WB1_TOKEN`、`WB2_TOKEN` 在聚合 Secret 尚未部署时仍可作为云端回退。
- 账号卡片会显示真实积分、今日奖励和连续签到天数；打开页面或点击“刷新积分 / 刷新全部积分”时，仅调用 WorkBuddy 的只读状态接口，不会执行签到。

## 智谱清言多账号

侧栏中的 **智谱** 页面支持管理智谱清言（chatglm.cn）多个账号的每日登录积分云端签到与积分监控。签到就是你打开清言客户端时提示的「签到 +200 积分」（每日登录奖励）。

- 点击「导入当前账号」直接从本机智谱清言客户端读取登录态（`%APPDATA%\chatglm\Network\Cookies` 中的 `chatglm_token` / `chatglm_refresh_token`，明文列直读）；客户端不可用时可在对话框中展开手动粘贴 token。
- 导入时在线验证登录态（认证失败拒绝保存，网络失败仍保存并提示稍后确认）；token 以 AES-256-GCM 加密保存在本机，界面只显示用户标识不显示 token。
- 账号卡片显示当前积分与活动状态；查询只调用 `score_activity_status` 只读接口，本地绝不执行签到。
- “立即签到”通过 `gh workflow run` 触发 daily-checkin 仓库的工作流（`account_filter=zhipu:<账号ID>`），云端调用 `daily_login_score` 完成领取（2026-09-08 抓包验证）。
- **云端全自主续期**：access token 约 24 小时有效，但云端每次签到前会用 refresh token（约 180 天有效且不轮换，已逆向刷新接口签名算法）自动换新 access token——本地导入一次后 **180 天内无需任何维护**，到期前在工具里重新导入即可。

## 最近修复

- 统一 Work CN 的平台别名和客户端路径识别，避免重启后账号消失或切换时找不到数据。
- WorkBuddy 认证写入失败时自动回滚，避免留下半保存状态。
- 区分 WorkBuddy 账号切换成功与窗口激活失败，错误提示不再误报。
- 统一 WorkBuddy 卡片 metrics 数据结构，修复积分和状态展示异常。
- 补充 Work CN 会话监听器的异常清理，降低重复监听和状态错乱风险。
- 同步前端锁文件版本，并补充切换、认证回滚、卡片数据和生命周期回归测试。

## 验证状态

- `npm test`：45 项通过。
- `npm run typecheck`：通过。
- `npm run build`：通过。
- Rust 单元测试：277 项通过，3 项忽略（其中智谱相关 19 项）。
- 云端 daily-checkin 仓库：zhipu signer 自检通过；真实账号实测签到链路返回「今日已领取，当前积分 948」。

以上结果对应当前修复快照；发布前仍应在目标 Windows 环境验证 Tauri 启动和安装包行为。

## 支持平台

- Windows 11（第一版仅保证 Windows）

## 致谢

- 上游项目：[Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) by jlcodes
- 上游许可证：CC-BY-NC-SA-4.0

## 许可证

本项目继承上游许可证 **CC-BY-NC-SA-4.0**（非商业性使用 · 相同方式共享）。
