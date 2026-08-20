# WorkBuddy 网络链路隔离设计

## 目标

- WorkBuddy 积分与签到状态接口始终直连国内服务，不读取 Clash/Windows 系统代理。
- GitHub Secret 同步先尝试国内直连；网络类失败时再使用当前 VPN 代理重试。
- 两条链路失败时给出准确提示，不把网络错误显示为账号或 Token 失效。

## 方案

WorkBuddy 状态查询使用独立的 `reqwest` 客户端，并调用 `no_proxy()` 禁用系统代理。GitHub 同步继续通过现有 `gh` 抽象执行，但真实 runner 增加网络路由策略：第一次执行清除代理环境变量；如果返回可识别的连接/超时/TLS/代理错误，则使用继承的代理环境或 Windows 系统代理再次执行。非网络错误（未登录、参数错误、权限错误）不重复提交。

如果直连失败且没有可用 VPN 代理，错误明确提示用户开启 VPN 后重试。不会把 GitHub Secret 发送到第三方镜像服务。

## 错误语义

- HTTP 401：WorkBuddy 认证失效，提示重新登录并导入。
- DNS、连接、超时、TLS、代理错误：WorkBuddy 网络失败，提示检查 VPN/网络。
- GitHub 直连失败且代理不可用：提示开启 VPN。
- GitHub 代理重试仍失败：提示 VPN 代理连接失败，并保留脱敏错误。

## 验证

- Rust 单元测试验证 WorkBuddy 客户端构建为直连模式。
- Rust 单元测试验证 GitHub runner 仅对网络失败执行代理重试，并保持 Secret 只通过 stdin。
- 运行 WorkBuddy 定向 Rust 测试、前端类型检查、生产构建和 NSIS 打包。
