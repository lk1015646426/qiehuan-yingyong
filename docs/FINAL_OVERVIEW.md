# TRAE Work CN 四账号切换器 — 最终交付总览

> 仓库：`C:\Users\10156\Desktop\脚本\切换应用\source`（cockpit-tools v1.3.16）
> 分支：`codex/trae-work-cn-switcher`
> 形态：Tauri v2 桌面应用（Rust 后端 + React 19 前端），内部平台标识 `TraeSoloCn`，UI 显示「TRAE Work CN」

## 已交付的全部阶段（0–8，TDD 完成、已提交、已打 tag）

| 阶段 | 能力 | commit | tag |
| --- | --- | --- | --- |
| 0–1 | 基线 + 品牌/应用壳收敛 | `4a2d304f` | work-cn-stage-1 |
| 2 | Work CN 路径/安装发现（注册表、EXE、数据目录、版本） | `1a59a03b` | work-cn-stage-2 |
| 3 | 账号快照导入（设备字段、密钥对提取、4 账号上限、AES 落盘、脱敏） | `84d5fd29` | work-cn-stage-3 |
| 4 | 一键切换与回滚（状态机 + 全局锁 + 注入链路 + verify + rollback） | `48223bb6` | work-cn-stage-4 |
| 5 | 积分查询与展示（纯函数解析，**仅查不领**） | `f177ff48` | work-cn-stage-5 |
| 6 | GitHub Secrets 同步（runner 抽象 + fake runner + JWT exp + stdin 写 secret） | `741ce86f` | work-cn-stage-6 |
| 7 | 后台会话监测 + 四账号完整体验（60s watcher + mtime 门禁 + 退避 + GitHub 接线） | `7ab5459b` | work-cn-stage-7 |
| 8 | 清理/发布/安装包（多平台死代码清除、updater 关闭、NSIS 单目标、死依赖移除） | 见 tag | work-cn-stage-8 |

## 核心能力一览
- **多账号管理**：导入至多 4 个 TRAE Work CN 账号（按 user_id 去重），凭证 AES-256-GCM 加密落盘，视图脱敏。
- **一键切换**：状态机串行锁 → 校验 → 同步本地会话 → 快照 → 注入 storage.json → 绑定 → 启动 → 验证 → 失败自动回滚。
- **积分查询**：解析 `ide_user_ent_usage`，支持多包求和、无限包、隐藏包跳过、剩余不为负；刷新仅走用量刷新接口，**绝不签到/领取**。
- **GitHub 同步**：把每个账号的 `access_token` / `checkin_device_id` 写入仓库 Secrets `TRAE{N}_TOKEN` / `TRAE{N}_DEVICE_ID`（secret 经 stdin，不进命令行）；过期 token / 缺设备 ID 自动跳过，未登录报硬错误。
- **路径发现**：原生 winreg 枚举卸载项定位 EXE 与数据目录，读取 EXE 旁 `product.json` 版本。

## 验收证据（本次开发内完成）
- 单元测试：`cargo test --lib work_cn` → **53 passed / 0 failed**（含切换状态机、watcher、GitHub 同步、积分解析）。
- Rust 编译：`cargo check --no-default-features -p cockpit-tools` → **0 error**。
- 前端类型：`npm run typecheck` → **exit 0**；`npm run build`（vite 生产构建）→ **成功**。
- 安装包：`npm run tauri build` → NSIS 当前用户安装包（`src-tauri/target/release/bundle/nsis/`，实际输出目录以本机构建配置为准）。

## 阶段 8 清理摘要
- 前端仅保留 Work CN 闭环 11 个文件；后端 modules 收敛至 33 个、commands 收敛至 5 个；`generate_handler!` 白名单化。
- updater 彻底关闭（Cargo 依赖 + capabilities 权限 + tauri.conf 插件块全清）；移除 5 个死依赖（reqwest 0.13 补丁、minisign-verify、tokio-tungstenite、reqwest_dav、zip）。
- `modules/account.rs` 裁掉 875 行之后的多平台自动切换/配额告警/双路切换死代码链（外部仅依赖 get_data_dir/resolve_data_dir/is_dev_profile）。
- bundle 目标仅 `nsis`（currentUser），resources 仅保留 trae-solo-cn.png，deep-link 收敛为 cockpit-tools。
- 新增 `clear_work_cn_credentials` 命令 + 设置页「清除本地凭证」危险按钮；卸载不触碰用户数据目录。

## 如何运行 / 人工端到端验收
```bash
cd C:/Users/10156/Desktop/脚本\切换应用/source
npm install          # 仅需首次
npm run tauri:dev    # 启动开发版（注意是 tauri:dev，不是 tauri dev）
```
- 阶段 2 路径发现已通过 `tauri:dev` 运行时验收（日志 `installation detected: installed=true`）。
- 阶段 4 真实切换、阶段 6 真实 GitHub 同步的端到端效果，**需你在本机 `npm run tauri:dev` 手动验收**（自动化覆盖状态机、JWT 解析、配置校验、fake runner 下的 secret 写入与跳过/错误分支；真实 `gh` 退出码、网络、GitHub 403 等仅在手动验收体现）。
- 使用 GitHub 同步前，请先在本机 `gh auth login` 并完成 `gh auth status`。

## 已知限制
- GitHub 同步仅写入 Secrets，不触发任何签到/领取/计费动作（符合硬约束）。
- Secret 名固定 `TRAE{N}_TOKEN` / `TRAE{N}_DEVICE_ID`，与 CI 约定一致；改名需同步 `work_cn_github.rs`。
- 4 个 slot 上限与账号 4 上限一致，slot 与 account 一一对应，重复绑定被 `validate_github_config` 拒绝。

## 文档位置
- 总纲：`TRAE_WORK_CN_SWITCHER_GLM52_DEVELOPMENT_GUIDE.md`
- 各阶段报告：`source/docs/STAGE0_BASELINE_REPORT.md` … `STAGE6_REPORT.md`
