# TRAE Work CN 账号切换器

一款 **Windows 专用**的 [TRAE Work CN](https://www.trae.cn/) 多账号管理工具，支持最多 4 个积分制账号的一键切换与免登录打开。

> 本项目基于开源项目 [Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) `v1.3.16` 改造而来，复用其成熟的 TRAE 账号管理与切号实现。详见 [NOTICE.md](./NOTICE.md)。

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

主指标为 **总积分 / 已用积分 / 剩余积分**；积分签到继续由现有 GitHub Actions 完成，本软件只查询积分、维护登录快照并同步签到凭证。

## WorkBuddy 多账号

侧栏中的 **WorkBuddy** 页面与 TRAE Work CN 账号库完全隔离，支持导入任意数量的官方客户端登录快照、加密保存、切换并启动客户端，以及选择参与云端自动签到的账号。

- 首先在官方 WorkBuddy 客户端完成正常登录，再导入当前账号；工具不会自动输入验证码、扫码或绕过登录限制。
- 认证快照只保存在本机并使用 AES-256-GCM 加密。GitHub 仅接收签到需要的 access token，绝不接收完整认证文件、refresh token、session state 或完整手机号。
- 切换会完整原子替换认证文件。客户端未能正常退出时，必须在确认后才会强制结束进程；取消不会修改认证文件。
- 默认自动检测 WorkBuddy；可在“路径设置”中填写 EXE 或认证文件路径。认证文件路径会同时用于导入、切换与后台凭证监测。
- 启用“参与自动签到”的账号会同步到一个 `WORKBUDDY_ACCOUNTS_JSON` GitHub Secret。旧的 `WB1_TOKEN`、`WB2_TOKEN` 在聚合 Secret 尚未部署时仍可作为云端回退。
- 账号卡片会显示真实积分、今日奖励和连续签到天数；打开页面或点击“刷新积分 / 刷新全部积分”时，仅调用 WorkBuddy 的只读状态接口，不会执行签到。

## 支持平台

- Windows 11（第一版仅保证 Windows）

## 致谢

- 上游项目：[Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) by jlcodes
- 上游许可证：CC-BY-NC-SA-4.0

## 许可证

本项目继承上游许可证 **CC-BY-NC-SA-4.0**（非商业性使用 · 相同方式共享）。
