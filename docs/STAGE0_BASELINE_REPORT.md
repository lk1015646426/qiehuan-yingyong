# 阶段 0 基线验证记录

验证日期：2026-08-12

## 源码

- 上游：jlcodes99/cockpit-tools
- 标签：v1.3.16
- 源码提交：e1ef55ce9f158dd1ee9fd682cf8d9aa1b79601e8
- 开发分支：codex/trae-work-cn-switcher

## 已通过

- `npm install`：成功；安装 181 个 package。
- `npm run typecheck`：退出码 0。
- `cargo check -p cockpit-tools`：退出码 0；耗时约 13 分 28 秒；上游基线产生 239 个 warning，无编译错误。
- `npm run tauri dev`：成功编译并启动 `target/debug/cockpit-tools.exe`。
- Vite：监听 `http://127.0.0.1:1420/`。

## 环境处理

上游 build.rs 会编译 Go sidecar。默认 `proxy.golang.org` 在当前网络超时，临时使用：

```powershell
$env:GOPROXY='https://goproxy.cn,direct'
$env:GOSUMDB='sum.golang.google.cn'
```

sidecar 已生成：

```text
sidecars/cockpit-cliproxy/bin/cockpit-cliproxy-x86_64-pc-windows-msvc.exe
```

后续本机开发可使用：

```powershell
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
```

跳过重复编译，但仅在上述文件存在时生效。

## 完整 Rust 测试基线问题

运行 `cargo test -p cockpit-tools` 时：

1. 第一次因 `proxy.golang.org` 超时，Go sidecar 构建失败；代理调整后已解决。
2. 第二次在生成超大的 Rust 测试目标时，`rustc` 以 Windows 状态 `0xC0000409 (STATUS_STACK_BUFFER_OVERRUN)` 异常退出；没有出现测试断言失败或 Rust 源码编译错误。
3. 随后 `cargo check -p cockpit-tools` 已成功，开发版也已成功启动。

这属于当前上游完整测试目标/本机 Rust 工具链的基线异常。后续每阶段至少必须运行：

```powershell
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
```

并优先运行与本次修改相关的定向 Rust 测试。不要把当前完整 `cargo test` 异常误归因于后续业务改动；如果工具链或测试拆分后可恢复，应重新启用完整测试。

## npm audit

`npm install` 报告上游依赖存在：

- 1 个 low
- 1 个 moderate
- 6 个 high

阶段 0 不运行 `npm audit fix`，因为自动升级可能破坏 v1.3.16 固定基线。应在功能稳定后单独评估依赖升级。
