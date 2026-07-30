# P5-F191：HTTP Package Schema Boundary 投影

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 问题与目标

真实 Account `ping` 的 `std.http.HttpRequest/HttpResponse` 已正确解析为 PackageSymbol，但
compiler boundary projection 对 PackageSymbol 一律返回 `UnsupportedBoundaryType`。让已验证的 std
HTTP Package schema 类型进入 unary/server-stream boundary contract，并保持普通不可传输 Package 类型
失败关闭。

不得恢复 builtin/native HTTP 特例，不得复制 service-owned schema。

## 验证

- 现有 `imported_http_types_reach_unary_and_stream_boundary_projection` 通过；
- Account ping 真实 PackageArtifact/ServiceContract/deployment 通过；
- 未声明依赖、未知类型、不可传输 record/interface 继续拒绝；
- compiler/deployment 聚焦测试、workspace check、diff check；
- 独立提交和 result。

