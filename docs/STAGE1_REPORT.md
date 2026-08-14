# 阶段 1 验收报告

- 验收日期：2026-08-13
- 分支：`codex/trae-work-cn-switcher`
- 基线：`e1ef55ce9f158dd1ee9fd682cf8d9aa1b79601e8`
- 验收范围：实施计划 Task 1；未进入 Task 2
- 结论：阶段 1 自动验证与开发版启动验收通过；等待用户确认后再决定是否提交

## 已实现

- 产品名为 `TRAE Work CN 账号切换器`，npm/Rust/Tauri 版本为 `0.1.0`。
- 正式 identifier 为 `com.lk.trae-work-cn-switcher`，开发 identifier 为 `com.lk.trae-work-cn-switcher.dev`。
- 主窗口为 960×680，最小尺寸为 820×560。
- `App.tsx` 只渲染 Work CN 应用壳；页面包含客户端检测占位、4 个空账号槽、设置与日志入口。
- 上游 updater endpoint 已移除，`createUpdaterArtifacts=false`；保留插件初始化所需的公开验证公钥，并将 endpoint 固定为空数组。
- 正式/开发数据目录分别为 `%USERPROFILE%\.trae_work_cn_switcher` 与 `%USERPROFILE%\.trae_work_cn_switcher_dev`；debug 构建强制使用开发目录。
- 开发脚本在 Windows 上通过项目本地 `npm.cmd` / `npx.cmd` 启动 Tauri CLI，并显式加载 `src-tauri/tauri.dev.conf.json`。
- 移除应用壳对 Google Fonts 的运行时依赖，避免离线或字体 CDN 异常导致前端控制台错误；保留系统字体回退。
- README 与 NOTICE 保留 Cockpit Tools v1.3.16、原作者和许可证署名。

## 最终自动验证

以下结果均来自最终代码状态下的新鲜运行。

| 命令 | 退出码 | 结果 | 关键输出 | 运行时间 |
|---|---:|---|---|---:|
| `node --test scripts/tests/work-cn-stage1.test.mjs` | 0 | 通过 | 4 tests，4 pass，0 fail | 0.44 秒 |
| `npm run typecheck` | 0 | 通过 | `tsc --noEmit` 无类型错误 | 65.94 秒 |
| `npm run build` | 0 | 通过 | Vite 7.3.1，86 modules transformed，生成 `dist/` | 135.03 秒 |
| `$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'; cargo check -p cockpit-tools` | 0 | 通过 | `Finished dev profile`；239 个上游 warning；0 error | 2.86 秒 |

阶段 1 定向测试现在覆盖：

1. 正式/开发数据目录与原 Cockpit Tools 隔离；
2. package-lock 根元数据与 package.json 对齐；
3. updater 配置合法但无任何上游 endpoint；
4. 应用壳不依赖远程字体资源。

`package-lock.json` 最终仍为 `4` 行新增、`4` 行删除。

## 开发版启动验收

### 实际启动命令

PowerShell 中使用：

```powershell
$env:GOPROXY='https://goproxy.cn,direct'
$env:GOSUMDB='sum.golang.google.cn'
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_BUILD_JOBS='1'
$env:CARGO_INCREMENTAL='0'
npm run tauri:dev
```

说明：实施计划写的是 `npm run tauri dev`，但 npm 会把它解析为 `tauri` 脚本加参数 `dev`，不会执行仓库的 `tauri:dev` 脚本，也不会加载 `tauri.dev.conf.json`。实际验收使用 package.json 中定义的 `npm run tauri:dev`。该命令是长驻开发进程，没有自然退出码；窗口成功启动后完成验收，再主动结束开发进程。结束后项目相关进程数为 0，1420 端口监听数为 0。

### 人工验收结果

| 项目 | 结果 | 观察证据 |
|---|---|---|
| 窗口标题 | 通过 | 窗口标题精确为 `TRAE Work CN 账号切换器 Dev` |
| 页面范围 | 通过 | 只显示 Work CN 应用壳；无 Cockpit Tools 其他平台导航 |
| 四个账号槽 | 通过 | 可访问性树与截图均显示账号 1～4，全部为“空槽位 · 可导入” |
| 设置/日志入口 | 通过 | 页面右上角显示“设置”“日志”按钮 |
| 开发数据目录 | 通过 | `%USERPROFILE%\.trae_work_cn_switcher_dev` 存在 |
| 正式数据目录隔离 | 通过 | `%USERPROFILE%\.trae_work_cn_switcher` 不存在 |
| 原 Cockpit Tools 开发目录隔离 | 通过 | `%USERPROFILE%\.antigravity_cockpit_dev` 不存在 |
| 前端控制台 | 通过 | 全新启动后 DevTools Console 只有 React DevTools 提示，0 个 error |
| 当前启动日志 | 通过 | 当前成功进程启动后未新增 panic/updater 初始化错误 |

开发版进程路径确认是：

```text
C:\Users\10156\Desktop\脚本\切换应用\source\target\debug\cockpit-tools.exe
```

已安装的原 Cockpit Tools 未被修改、关闭或替换。

## 验收中发现并修复的问题

1. 删除整个 `plugins.updater` 配置后，代码仍无条件初始化 updater 插件，启动时因配置为 `null` panic。
2. 空 updater 对象仍缺少必填 `pubkey`。最终保留公开验证公钥、设置 `endpoints: []`，既满足插件初始化，也不会连接上游更新源。
3. `scripts/tauri-dev.cjs` 在 Windows 上直接调用裸 `npm` / `tauri`，本机仅有 `.cmd` 入口，导致脚本静默退出 1。已改为平台化调用项目本地 `npm.cmd` / `npx.cmd`，并显式检查 spawn 错误。
4. `src/styles/base.css` 的 Google Fonts 运行时请求返回 404，导致控制台出现 1 个前端资源错误。已移除远程字体 import，使用已有系统字体回退。

以上修复均在 Task 1 范围内完成，并先增加能复现问题的定向测试，再做最小修改。

## 已知基线问题

- Rust stable 为 `rustc 1.96.1`。本机没有配置 Windows 分页文件；默认开发代码生成曾出现 `memory allocation ... failed`，随后 rustc 以 `0xC0000409 (STATUS_STACK_BUFFER_OVERRUN)` 退出。使用 `CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_BUILD_JOBS=1`、`CARGO_INCREMENTAL=0` 后，`cargo build -p cockpit-tools` 退出码 0，耗时 769.58 秒，开发版可以启动。该问题属于当前大 crate/工具链/内存提交环境，不是阶段 1 TypeScript 或 Rust 源码编译错误。
- 一次 `cargo check -p cockpit-tools` 曾在同一 rustc 暂态异常中退出 101；原样重试后通过，最终新鲜检查退出码为 0。
- 当前 Rust check 仍有上游 239 个 warning。本阶段没有解决或隐藏这些 warning。
- `cargo fmt --check` 退出码 1：当前 stable 工具链未安装 `cargo-fmt.exe`。本阶段未擅自安装 rustfmt。
- 完整 `cargo test -p cockpit-tools` 本阶段未重跑。阶段 0 已记录其在大测试目标上以 `0xC0000409` 异常退出，没有测试断言失败证据，因此不能声称完整 Rust 测试通过。
- 应用日志仍保留本轮修复前两次 updater 初始化失败记录（02:08、02:19）；最终成功进程从 02:52 后未新增同类错误。报告不删除历史诊断日志。

## 修复过程中的关键失败命令

这些失败是定位问题的真实证据，不是最终验收结果：

| 命令 | 退出码/状态 | 原因 |
|---|---:|---|
| `$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'; cargo check -p cockpit-tools`（首次） | 101 | rustc 暂态元数据/内存异常，底层为 `0xC0000409`；原样重试通过 |
| `cargo fmt --check` | 1 | `cargo-fmt.exe` 未安装 |
| `npm run tauri dev`（删除 updater 配置后） | 101 | updater 配置为 `null`，插件初始化 panic |
| `npm run tauri dev`（空 updater 对象后） | 101 | updater 配置缺少必填 `pubkey` |
| `npm run tauri:dev`（开发脚本修复前） | 1 | Windows 裸 `npm`/`tauri` 命令不可执行，脚本未回显 spawn error |

## 下一步建议

1. 等用户确认后再提交阶段 1；当前不 commit、不 push、不发布。
2. 在进入 Task 2 前，建议启用系统管理的 Windows 分页文件，或把低内存 Cargo 环境固化为明确的本机开发说明，避免大 crate 代码生成再次出现 `0xC0000409`。
3. 可在用户同意后安装 rustfmt，并执行 `cargo fmt --check`；不要把安装组件与 Task 2 功能开发混在同一提交。
4. Task 2 仍应按实施计划从 Work CN 新旧安装名称和路径检测开始，不提前实现账号导入、切换、积分或 GitHub Secrets。

## 安全确认

- 未读取真实 TRAE `storage.json`。
- 未读取或输出真实 Token、refresh token、私钥、GitHub PAT 或 GitHub Secrets。
- 未修改 `C:\Users\10156\AppData\Local\Cockpit Tools`。
- 未提交、push、发布或进入 Task 2。
