# P5-F174：Test Runner Package Schema Cutover

状态：Ready

## 直接父任务

- `P5-F172-runtime-eval-websocket-package-schema-result.md`

## 当前断点

workspace已越过runtime eval，test-runner的ecosystem smoke和package-test assembly仍构造
`ServiceContractDefinition.boundary_schema`，且调用deployment projection时未传Package schema records。

## 范围

只修改`test-runner`及其测试，并写result。

## 必须实现

- contract definition改用`package_type_requirements`，删除service-owned schema。
- deployment projection显式传入与fixture Package artifacts一致的schema records。
- 无命名边界类型的fixture使用合法空requirements/records；存在命名类型时使用声明Package的真实
  canonical records，不能用空值绕过。
- 保持package-test overlay、service selector、WebSocket smoke和deployment identity语义。

## 验证

- `cargo test -p skiff-test-runner`；
- test-runner旧`boundary_schema`/旧类型符号无命中；
- `cargo check --workspace`首错越过test-runner；
- `git diff --check`；
- 独立提交并写result。
