# NOTICE

本项目 **切换应用**（`qiehuan-yingyong`）是
[Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) 的派生作品。

## 上游来源

- 上游仓库：https://github.com/jlcodes99/cockpit-tools
- 上游作者：jlcodes
- 固定基线版本：`v1.3.16`（源码提交 `e1ef55ce9f158dd1ee9fd682cf8d9aa1b79601e8`）
- 上游许可证：CC-BY-NC-SA-4.0

## 派生说明

本项目在上游 Cockpit Tools 的基础上进行改造，目标是构建 Windows 专用的
切换应用的 Windows 多账号管理体验。主要改动包括：

- 收敛产品品牌与应用壳为“切换应用”；
- 隐藏 Cockpit Tools 原有的多平台入口，仅保留 TRAE Work CN 相关界面；
- 关闭指向上游 Cockpit Tools 发布源的应用内更新通道。

上游的 TRAE 账号管理、切号调用链、`storage.json` 原子写入、进程管理等
核心实现均予以复用，未重新实现。

## 许可证

本项目继承上游许可证 **CC-BY-NC-SA-4.0**（非商业性使用 · 相同方式共享）。

## 归属

- 上游作者：jlcodes
- 派生项目作者：lk
