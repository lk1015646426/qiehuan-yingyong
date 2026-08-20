# 发布流程（TRAE Work CN 切换工具 / Windows NSIS）

> 本 fork 唯一发布目标：Windows x64 NSIS 安装包。上游的 macOS dmg / Homebrew Cask / Linux / updater 流程**均不适用**（bundle targets 仅 `["nsis"]`，`createUpdaterArtifacts: false`）。

## 1. 前置检查

1. 版本三处一致：`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`（当前 0.1.2；可用 `npm run sync-version` 对齐）。
2. 基线验证：
   ```powershell
   npm run typecheck                    # 前端类型零错误
   cargo test --manifest-path src-tauri/Cargo.toml   # 全绿
   node --test src/utils/workBuddyLifecycle.test.ts  # WorkBuddy 页面契约
   ```
3. 确认无未提交改动（`git status`），本次要发布的内容已按逻辑提交。
4. 环境要求：Node 20+、Rust stable-msvc、Windows。

## 2. 构建

```powershell
npm run tauri -- build --ci
```

产物：`target\release\bundle\nsis\切换工具_<version>_x64-setup.exe`

- NSIS `installMode: currentUser`，无需管理员（gh 安装等操作中 UAC 由系统按需弹出）。
- `installer-hooks.nsh` 会在安装时自动结束旧版本进程，支持覆盖安装。

## 3. 校验与冒烟

```powershell
Get-FileHash "target\release\bundle\nsis\切换工具_<version>_x64-setup.exe" -Algorithm SHA256
```

装机冒烟清单（人工）：

1. 覆盖安装旧版本 → 旧进程被杀、安装完成、启动正常。
2. 账号列表与 GitHub 配置在升级后保留。
3. 切换账号 → 客户端打开且身份正确。
4. 云端签到页：gh 徽标状态正确；同步全部 GitHub 正常（或引导弹窗正常弹出）。
5. 点 × → 最小化到托盘；托盘退出可用。
6. 全程无黑色命令行窗口闪烁。
7. WorkBuddy 页面：刷新全部积分后，每个账号显示真实积分、今日奖励和连续签到；查询不能触发签到，卡片保持紧凑且无内容重叠。

## 4. 发布

1. 按 [release-template.md](release-template.md) 起草 Release Notes（Windows 下载段 + SHA256；如杀软误报附 VirusTotal 链接与 [false-positive-template.md](false-positive-template.md) 工单进展）。
2. 用户确认后再执行 git tag / GitHub Release / 分发安装包；**不得擅自 push 或发布**。

## 5. 杀软误报处理

无代码签名的 NSIS 包可能被部分引擎误报：先提交厂商误报工单（模板见 false-positive-template.md），在 Release Notes 中说明；不为此引入签名证书除非用户明确要求。
