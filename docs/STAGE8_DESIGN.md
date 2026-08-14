# 阶段 8 增量设计：清理、发布和安装包

> 项目：TRAE Work CN 四账号切换器（cockpit-tools v1.3.16，分支 `codex/trae-work-cn-switcher`，HEAD=`7ab5459b`）
> 目标：把多平台账号管理器 **特化为只服务 TRAE Work CN 单平台**，并产出可发布的安装包。
> 本设计只覆盖阶段 8。工程师**只做删除/收敛，不新增业务逻辑**（除 §4 列出的极小发布配置改动）。

---

## 0. 现状结论（已探索，直接采信）

1. **前端入口已收敛**：`src/App.tsx` 仅 `return <WorkCnSwitcherPage />`（阶段 1 已做），旧页面/组件/路由**全部已不可达**，只是文件还在。前端清理 = 物理删除死代码。
2. **后端未收敛**：`src-tauri/src/lib.rs` 的 `generate_handler!` 注册了约 700 条命令（40+ 平台）；`.setup()` 拉起多平台后台任务（`provider_token_keeper`、`auto_local_import`、多个 OAuth 监听、wakeup/codex/websocket 等）；`tray.rs` 菜单引用全部平台的账号模块。
3. **发布配置已部分定制**：`tauri.conf.json` 的 `productName`/`identifier`/NSIS `currentUser` 已就位；但 updater 插件仍注册、`resources` 仍打包 16 个平台图标、`deep-link` 仍含 `zcode` scheme、`externalBin` 仍带 `cockpit-cliproxy`。

> 方法论：**`cargo check` / `npm run typecheck` 是删除的安全网**。删除任何模块/文件后立刻编译，编译报错 = 仍有引用 = 还原该文件或在引用处一并处理。严格按 §5 的小组顺序「删一组 → 验一组」，绝不一次性全删。

---

## 1. 清理范围清单

### 1.1 前端 `src/`（A 必删 / B 必留 / C 通用保留）

**B + C —— 必留（入口可达闭包，共 11 个文件）**：

```
src/main.tsx
src/App.tsx
src/App.css
src/i18n/index.ts                       # main.tsx initI18n + AppRuntimeGuard 引用
src/components/AppRuntimeGuard.tsx      # main.tsx 引用
src/components/work-cn/WorkCnAddAccountDialog.tsx
src/components/work-cn/WorkCnSettingsDialog.tsx
src/components/work-cn/WorkCnStatusBanner.tsx
src/pages/WorkCnSwitcherPage.tsx
src/stores/useWorkCnStore.ts
src/services/workCnService.ts
src/types/workCn.ts
src/utils/errorReporter.ts              # main.tsx 引用；其内部 invoke diagnostics_* 命令
```

（`errorReporter.ts` 内部仅 `import { getVersion } from '@tauri-apps/api/app'` + `invoke`，无平台耦合，安全保留。）

**A —— 必删（整目录/整子树，均不可达）**：

```
src/pages/           除 WorkCnSwitcherPage.tsx 之外全部（39 个页面）
src/components/      除 AppRuntimeGuard.tsx 与 work-cn/ 之外全部（含 icons/、layout/SideNav.tsx、
                     platform/、codex/、codebuddy/、codebuddy-suite/、easter-egg/、model-provider/）
src/stores/          除 useWorkCnStore.ts 之外全部（~40 个）
src/services/        除 workCnService.ts 之外全部（~50 个）
src/types/           除 workCn.ts 之外全部（~28 个）
src/hooks/           全部删除（useAutoRefresh.ts 等，入口不可达）
src/utils/           除 errorReporter.ts 之外全部（~70 个，含各平台工具/test 文件）
```

**要点**：
- 前端死代码**互相引用**（如 `AccountsPage → useAccountStore → accountService → types/account`），必须**整批删除**，不能单个文件删——否则 `tsc` 会因死文件间的悬空 import 报错。做法：按 1.1 的「必留清单」白名单，删除白名单之外的一切，然后 `npm run typecheck`。
- `App.tsx` 注释里写明「旧页面暂时保留」——阶段 8 正是执行这个物理裁剪。

### 1.2 后端模块 `src-tauri/src/modules/`（B 必留 / A 必删 / 待验证）

**B + C —— 必留（Work CN 依赖闭包 + 通用基础设施，共 33 个）**：

```
account                 # trae_account 的 get_data_dir 依赖
account_index_repair    # trae 索引修复
app_lifecycle           # 关机监听（lib.rs）
atomic_write            # 禁止删除
config                  # 用户配置
deferred_account_rewrite# trae_account load_account 依赖
diagnostics             # panic hook / 前端就绪看门狗（lib.rs）
i18n                    # 前端 + 托盘文案
instance                # trae_instance 依赖
instance_store          # trae_instance 依赖
logger                  # 禁止删除
main_window_state       # 窗口几何（lib.rs）
oauth                   # trae_oauth 依赖
oauth_pending_state     # trae_oauth 依赖
oauth_server            # trae_oauth 依赖
process                 # 禁止删除；trae 关闭/启动依赖
process_memory          # 窗口关闭到托盘后的内存裁剪（lib.rs）
process_timeout         # trae_account line 2308 依赖
provider_current_state  # trae_account line 4583 依赖（泛型，无平台耦合）
secure_account_storage  # 禁止删除
sync_settings           # lib.rs 启动合并语言设置
test_support            # #[cfg(test)]
trae_account            # 核心
trae_instance           # 核心
trae_oauth              # 核心（保留；若证实 orphaned 可再删）
webkit_cache_maintenance# lib.rs 启动清理 WebKit WAL
work_cn_github          # 核心
work_cn_session_watcher # 核心
```

**需裁剪但保留（TRIM，见 §1.4）**：`tray`、`tray_layout`、`floating_card_window`。

**A —— 必删（平台专属 + 功能专属，共 ~70 个，按组）**：

| 组 | 模块 |
|---|---|
| Codex | `codex_account` `codex_agent_identity` `codex_app_injection` `codex_config_format` `codex_instance` `codex_local_access` `codex_oauth` `codex_official_app_server` `codex_protocol` `codex_quota` `codex_session_file_time` `codex_session_manager` `codex_session_visibility` `codex_speed` `codex_ssh` `codex_thread_sync` `codex_wakeup` `codex_wakeup_scheduler` |
| Claude | `claude_account` `claude_desktop_gateway` `claude_instance` |
| CodeBuddy/WorkBuddy | `codebuddy_account` `codebuddy_cn_account` `codebuddy_cn_instance` `codebuddy_cn_oauth` `codebuddy_instance` `codebuddy_oauth` `codebuddy_session` `codebuddy_session_list` `codebuddy_session_transfer` `workbuddy_account` `workbuddy_instance` `workbuddy_oauth` `workbuddy_session_transfer` |
| Cursor/Grok/Kiro/Windsurf/Zed/Zcode/Qoder | `cursor_account` `cursor_instance` `cursor_oauth` `grok_account` `grok_instance` `grok_oauth` `kiro_account` `kiro_instance` `kiro_oauth` `windsurf_account` `windsurf_devin_oauth` `windsurf_instance` `windsurf_oauth` `zed_account` `zed_instance` `zed_oauth` `zcode_account` `zcode_instance` `zcode_oauth` `qoder_account` `qoder_instance` `qoder_oauth` |
| GitHub Copilot / Antigravity / 其它平台 | `github_copilot_account` `github_copilot_instance` `github_copilot_oauth` `antigravity_credential` `antigravity_legacy_instance` `antigravity_paths` `antigravity_switch_history` `hermes_auth` `openclaw_auth` `opencode_auth` |
| 功能模块（非 Work CN） | `announcement` `auto_local_import` `external_import` `group_settings` `import` `provider_token_keeper` `remote_config` `ssh_server` `update_checker` `vscode_inject` `vscode_paths` `wakeup` `wakeup_gateway` `wakeup_history` `wakeup_scheduler` `wakeup_verification` `web_report` `webdav_domain` `webdav_sync` `websocket` `linux_updater` `macos_native_menu` |

**待验证（删前 `cargo check`，报错即保留）**：`db`、`quota`、`quota_cache`、`local_secret_blob`、`trae_session_transfer`。它们可能是通用依赖（`db` 可能被 config/settings 使用；`quota*` 可能被积分展示使用；`trae_session_transfer` 属 trae 链）。处理办法：放进最后一组删除，编译报错就回填。

### 1.3 后端命令 `src-tauri/src/commands/`

**必留**：
```
commands/work_cn.rs            # 7 个命令（含 get_work_cn_session_watch_status）
commands/work_cn_github.rs     # 4 个命令
commands/trae_instance.rs      # trae_account 直接调用 trae_start_instance（且 stop/close 也在用）
commands/system.rs             # 前端 errorReporter 调用 diagnostics_capture_event /
                               # diagnostics_frontend_stage / diagnostics_frontend_ready（TRIM：见下）
```

**TRIM**：`commands/system.rs` 需**删除其中的平台专属命令**（`codex_ssh_*`、`codex_managed_lb_provider_id`、`codebuddy_list_local_session_files`、`set_claude_app_scan_roots`、`set_trae_app_scan_roots`、`set_codex_launch_on_switch`、`set_codex_local_access_entry_visible`、`scan_claude_desktop_launch_targets`、`set_wakeup_override`、`save_tray_platform_layout` 等），保留通用命令（`open_data_folder`、`diagnostics_*`、`handle_window_close`、`get_general_config`、`patch_general_config`、`show_main_window_and_navigate` 等）。删除后 `cargo check` 兜底。

**必删**（其余 ~42 个命令模块）：`account` `announcement` `antigravity_legacy_instance` `claude` `claude_instance` `codebuddy` `codebuddy_cn` `codebuddy_cn_instance` `codebuddy_instance` `codebuddy_session` `codex` `codex_instance` `cursor` `cursor_instance` `data_transfer` `github_copilot` `github_copilot_instance` `grok` `grok_instance` `group` `import` `instance` `kiro` `kiro_instance` `logs` `oauth` `provider_current` `qoder` `qoder_instance` `remote_config` `ssh_server` `trae` `update` `wakeup` `windsurf` `windsurf_instance` `workbuddy` `workbuddy_instance` `zcode` `zcode_instance` `zed`。

> 注意：`commands/trae.rs`（泛 Trae 平台命令）删除；`commands/trae_instance.rs` 保留（Work CN 切换链路依赖）。`models/` 同理只删平台模型，保留 `account/instance/trae/work_cn/quota(若引用)/token` 等。

### 1.4 `lib.rs` 收敛（编辑，不删除）

**`.setup()` 删除以下调用/块**：
- `provider_token_keeper::ensure_started`、`auto_local_import::ensure_started`（保留 `work_cn_session_watcher::ensure_started`）。
- OAuth 恢复块里除 `trae_oauth` 外的监听：`codex_oauth`/`windsurf_oauth`/`kiro_oauth`/`zed_oauth`（`trae_oauth::restore_pending_oauth_listener()` 保留）。
- `codex_local_access::restore_local_access_gateway`、`codex_app_injection::restore_running_profiles`。
- `wakeup_scheduler`/`codex_wakeup_scheduler` 线程块。
- `websocket::start_server`、`web_report::start_server`。
- `external_import::handle_external_import_args`（三处：single-instance / deep-link open_url / get_current / run-event；连同 zcode 深链处理 `handle_zcode_oauth_deep_links`）。
- Updater 插件初始化 `app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;`（保留 `tauri_plugin_process` 与 `tauri_plugin_autostart`）。

**`generate_handler!` 保留白名单**：仅 `commands::work_cn::*`、`commands::work_cn_github::*`、`commands::trae_instance::*`、`commands::system::*`（已 TRIM 后）。其余全部删除。

**`RunEvent::ExitRequested/Exit` 块**：删除 `codex_app_injection::stop_all()` 与 `codex_local_access::shutdown_*` 两处。

**托盘收敛（关键 TRIM）**：`tray.rs` 现引用全部平台账号模块。收敛为**单平台极简托盘**：
- 菜单项固定为：`打开 TRAE Work CN 切换器` + `退出`（可选加 `当前账号` 显示，取 `trae_account::list_work_cn_accounts()` + 当前绑定）。删除 `tray_layout` 平台矩阵、`PlatformId` 多平台解析、所有 `*_account::list_accounts*` 引用（只保留 `trae_account`）。
- 删除 `tray_layout.rs`（或留空壳）。`floating_card_window` 保留为通用「关闭到托盘/恢复主窗口」壳（不引用平台模块即可；若其引用了 `provider_current_state` 之外的多平台逻辑，一并裁剪）。

---

## 2. 风险清单（易误删的共享依赖，逐条「不可删」）

1. **`account`（modules/account）**：看似是「多平台账号库」，但 `trae_account` 的 `get_data_dir()` 直接依赖它。**不可删**。
2. **`provider_current_state`**：名字像「平台聚合」，但 `trae_account.rs:4583` 调用其泛型函数 `resolve_existing_current_account_id`（无平台耦合）。**不可删**。
3. **`process_timeout`**：`trae_account.rs:2308` 用其 `output_with_timeout`。**不可删**。
4. **`account_index_repair` / `deferred_account_rewrite`**：trae 账号索引修复与延迟重写，`trae_account` 强依赖。**不可删**。
5. **`commands::trae_instance`**：`trae_account::switch_work_cn_account` 直接调 `trae_start_instance`；删 `commands::trae_instance` 会导致 Work CN 切换编译失败。**不可删**。
6. **`commands::system` 的 diagnostics_* 命令**：前端 `errorReporter.ts` 启动即 invoke。**保留这 3 个命令**（整文件 TRIM 而非整删）。
7. **`process`（modules/process）**：既是「通用进程模块」（spec 明确保留），又含 `cliproxy` 引用。若裁剪 `process.rs` 中 codex 专属的 cliproxy 函数，需同步移除 `tauri.conf.json` 的 `externalBin`；否则**保留 `externalBin` 与 `process.rs` 原样**（sidecar 只是多打包一个文件，无功能影响）。
8. **`secure_account_storage` / `atomic_write`**：spec 明确禁止删除。**不可删**。
9. **默认实例逻辑 / 运行中账号保护 / 启动后验证**：都在 `trae_instance` + `trae_account` 内（`load_default_settings_for_platform`、`resolve_running_bound_account_contexts`、`verify_work_cn_switched_account`）。**删除平台模块时绝不碰这几个函数**。
10. **`i18n`**：前端 `initI18n()` 与 `AppRuntimeGuard` 都引用；托盘 `get_text` 也引用。**不可删**（即使托盘极简化，前端仍用）。
11. **`sync_settings`**：`lib.rs` 启动时 `merge_setting_on_startup("language", …)`。**不可删**。
12. **`tray` + `floating_card_window`**：承载「关闭到托盘/恢复主窗口/退出」生命周期，`lib.rs` 的 on_window_event 与 RunEvent 依赖。**先裁剪其多平台菜单引用，不要整删**（除非主理人明确拍板「去托盘、关闭即退出」）。
13. **上游作者署名**：清理时保留各保留文件头部的上游版权/署名注释，不得批量删除。

---

## 3. 发布配置方案

### 3.1 `src-tauri/tauri.conf.json` 字段变更

| 字段 | 现值 | 目标值 | 说明 |
|---|---|---|---|
| `identifier` | `com.lk.trae-work-cn-switcher` | 保留（见 §5 拍板） | 已是新标识 |
| `productName` | `TRAE Work CN 账号切换器` | 保留 | 已符合 |
| `bundle.targets` | `"all"` | `["nsis"]` | Windows 当前用户安装（去掉 dmg/msi/deb/appimage 等） |
| `bundle.windows.nsis.installMode` | `currentUser` | 保留 | 已满足「当前用户安装」 |
| `bundle.createUpdaterArtifacts` | `false` | 保留 | updater 产物已关 |
| `plugins.updater` | 含 pubkey + `endpoints:[]` | **整块删除** | 彻底关闭 updater |
| `bundle.externalBin` | `cockpit-cliproxy` | **删除**（与 §2.7 同步） | 前提是 `process.rs` 的 cliproxy 路径一并裁掉；否则保留 |
| `bundle.resources` | 16 个平台图标 + claude helper | 仅保留 `trae-solo-cn.png` | 删除 `claude-desktop-auth-helper.cjs` 与其余平台图标 |
| `plugins.deep-link.schemes` | `["cockpit-tools","cockpittools","zcode"]` | 仅保留 `["cockpit-tools"]` 或整块删除 | 删除 `zcode`；若保留 trae_oauth 则留一个 scheme |
| `bundle.icon` | 现有 9 个图标文件 | 保留路径，替换图标内容（§5 拍板） | 图标文件在 `src-tauri/icons/` |
| `app.windows[1]`（floating-card） | 存在 | 保留或删除（§5 拍板） | 若去悬浮卡片则删此窗口 + `floating_card_window` |

### 3.2 updater 关闭（两处同步）

- `tauri.conf.json`：删 `plugins.updater` 块（§3.1）。
- `lib.rs`：删 `.plugin(tauri_plugin_updater::Builder::new().build())?;` 与对应 `info!` 行；同时删 `commands::update`、`modules/update_checker`、`modules/linux_updater`。`Cargo.toml` 中 `tauri-plugin-updater` 依赖可在最后移除（可选）。

### 3.3 「安装包不带账号数据」与「卸载保留账号库」

- **不带账号数据**：账号库写盘在用户数据目录（`account::get_data_dir()`，运行时 `%APPDATA%` 下），不在安装目录、不在 `resources`，因此安装包天然不含账号数据。**无需额外配置**，只需确认 `bundle.resources` 不再含任何账号/数据文件（当前不含，已满足）。
- **卸载保留账号库**：NSIS `installMode: currentUser` 的卸载只删除安装目录，不触碰 `%APPDATA%` 用户数据。**确认 `src-tauri/windows/nsis/installer-hooks.nsh` 无删除用户数据目录的逻辑**（现有 hooks 用于安装钩子，非账号数据），即可满足。
- **「清除本地凭证」**：作为独立小功能提供（默认列为可选项，见 §5.6）。若纳入，做法：`commands/work_cn.rs` 新增 `clear_work_cn_credentials`（删除 `trae_accounts` 目录 + `github.json`），`WorkCnSettingsDialog` 增加「清除本地凭证」危险按钮（二次确认）。

---

## 4. 任务列表（有序、含依赖、每步验证命令）

> 验证命令缩写：
> - `F1` = `npm run typecheck`
> - `F2` = `npm run build`
> - `R1` = `cargo check -p cockpit-tools`（快速）
> - `R2` = `cargo test -p cockpit-tools`（全量测试）
> - `R3` = `npm run tauri build`（最终打包）

**T01 —— 前端死代码清扫**
范围：§1.1 必删清单（白名单之外的 pages/components/stores/services/types/hooks/utils）。
依赖：无。
验证：`F1` → `F2`（typecheck 先，build 后；typecheck 报错说明有必留文件被误删，按报错还原）。

**T02 —— lib.rs 收敛（命令注册 + setup）**
范围：§1.4 的 `generate_handler!` 白名单化 + setup 删除多平台后台任务与 updater 插件。
依赖：T01（前端不再引用被删命令后，命令才能删）。
验证：`R1`。注意：此步只删「注册行」与「setup 调用」，**模块文件先不删**，靠编译确认无残留引用。

**T03 —— 后端命令层删除 + system.rs 裁剪**
范围：§1.3 必删的 42 个命令模块 + `commands/system.rs` 平台命令裁剪 + `commands/mod.rs` 去注册。
依赖：T02。
验证：`R1`。

**T04 —— 托盘收敛 + 后端模块删除（分组）**
范围：
- 4a：`tray.rs` 极简化 + 删 `tray_layout.rs`（依赖 T02/T03，因为托盘现在引用的是已删模块）。
- 4b：删 §1.2 平台专属模块组（Codex/Claude/CodeBuddy/WorkBuddy/Cursor/Grok/Kiro/Windsurf/Zed/Zcode/Qoder/GitHubCopilot/Antigravity）。
- 4c：删 §1.2 功能模块组（announcement/auto_local_import/external_import/import/provider_token_keeper/remote_config/ssh_server/wakeup*/web_report/webdav*/websocket/update_checker 等）。
- 4d：删「待验证」组（db/quota/quota_cache/local_secret_blob/trae_session_transfer）——报错即回填。
依赖：T03。
验证：每小组删完 `R1`，全部删完 `R2`。

**T05 —— 发布配置 + 最终构建**
范围：§3.1 的 `tauri.conf.json` 变更；`Cargo.toml` 移除 `tauri-plugin-updater`（可选）；可选「清除本地凭证」命令（§3.3）。
依赖：T04。
验证：`R2` → `F2` → `R3`（等价 `npm ci && npm run typecheck && npm run build && cargo test -p cockpit-tools && npm run tauri build`）。

---

## 5. 待主理人拍板（均已给默认值）

1. **identifier**：默认保留现有 `com.lk.trae-work-cn-switcher`（已唯一、已含 `trae-work-cn-switcher` 语义）。如需更正式可用 `cn.trae.work.switcher` 或你的域名反写。
2. **图标**：默认**复用现有 `src-tauri/icons/` 全套图标**（暂作占位），仅替换 `resources` 里的平台菜单图标为 `trae-solo-cn.png`。如你后续提供正式图标，工程师再替换 `icons/*` 文件即可，不改变路径。
3. **安装器类型**：默认 `["nsis"]`（当前用户）。若需企业级 MSI 可加 `"msi"`，但 MSI per-user 需额外 WiX 配置，默认不做。
4. **托盘/悬浮卡片**：默认**保留极简托盘（打开/退出）** + 保留 `floating_card_window`（关闭到托盘/恢复窗口）。备选「去托盘、关闭即退出」更省事，但改变现有 UX——请确认用哪种。
5. **deep-link**：默认保留 `deep-link` 插件但 scheme 收敛为 `cockpit-tools` 一个（`trae_oauth` 在保留链上）。若确认 Work CN 全链路不走 OAuth 深链，可整块删除 `deep-link` 插件与 `trae_oauth`。
6. **「清除本地凭证」**：默认**纳入阶段 8**（一个命令 + 设置弹窗一个危险按钮，工作量小，满足总纲「提供单独清除选项」）。若你想严格只做清理不做新功能，可顺延。

> 除第 4、6 点外，其余按默认值执行即可，无需再等。第 4、6 点如调整，工程师按最终决定落地。
