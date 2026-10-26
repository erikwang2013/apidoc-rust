# Changelog

本项目记录所有值得注意的变更。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循语义化版本。

## [2.2.0] - 2026-10-26

### Added

- UIC-1 后端契约（Contract 1.0）：`/apidoc/ui/v1/*` 提供 capabilities、config、menus、
  detail、docMenus/docDetail、verifyAuth、调试（send/start/poll/stop）、分享记录、
  缓存、代码模板与批量导出，apidoc-ui 的同一份 Vue 构建可直接消费。
- 九种外部协议定义解析：`#[apidoc::definition]` 支持 OpenAPI、AsyncAPI、GraphQL、
  Proto、WSDL、EDMX、JSON Schema、JSON-RPC、XML-RPC；统一输出定义、消息与逐条校验
  诊断，并接入 UIC-1 detail（`definitions` / `validation`）。
- Vue 静态 UI：`ui` / `ui_path` 与 `ApidocConfig::release(title, ui_path)`，发布包默认
  使用 apidoc-ui 构建，内嵌原生 UI 保留为库默认；含 SPA 兜底、静态资源与路径逃逸防护。
- 服务端中继能力按实际编译 feature 上报：`capabilities.debug.relay` 不再出现未编译
  执行器，`debug-all` 构建上报全部 30 个协议 id（Rust 仅一元 `relay-unary`）。
- CHANGELOG 与 2.2.0 版本文档、架构/功能/生命周期/多协议图、社交预览图。

### Fixed

- DNP3 应用层按 IIN 之后的偏移解析对象头，修正点数 u16 范围溢出。
- BACnet/IP 发送合法的 ReadProperty 请求编码。
- 工业协议解析资源上限：DNP3 每条响应最多 10,000 点，IEC 60870-5-104 最多 10,000 行，
  防止小帧放大解析结果。
- 定义文件解析上限：单文件 2 MiB / 50,000 行、最多 64 个导入、导入深度 32 层；路径
  限定在声明方 crate 根内并校验软链 realpath。

### Changed

- workspace / crates 版本升至 2.2.0，Cargo.lock 同步更新。
- README（中文与英文）补齐 UIC-1、九格式定义、Vue UI、全量 relay 能力与安全上限说明。
