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

## 当前阶段

本项目按阶段逐步开发，当前处于 **阶段 1：品牌和应用壳收敛**。

- 已完成：产品品牌、应用壳、窗口尺寸收敛为 TRAE Work CN 专用。
- 进行中：客户端安装发现、完整账号快照导入、一键切换等能力将在后续阶段接入。

## 支持平台

- Windows 11（第一版仅保证 Windows）

## 致谢

- 上游项目：[Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) by jlcodes
- 上游许可证：CC-BY-NC-SA-4.0

## 许可证

本项目继承上游许可证 **CC-BY-NC-SA-4.0**（非商业性使用 · 相同方式共享）。
