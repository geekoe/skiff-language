# P5-F162：Compiler Package Schema Input

状态：Ready

## 直接父任务

- `P5-F161-package-schema-compiler-projection-result.md`

## 已闭合决策与依赖

- 用户选择：第一版boundary可达named types必须在owner Package的`api.yml`显式公开；不实现closure-only稳定键。
- F160已提供`CanonicalArtifactStore::resolve_package_artifact_schema`，返回严格验证的index/records。
- 本任务只建立compiler driver到projection/contract的schema事实通道，不实现最终ServiceContract投影。

## 范围

修改compiler projection input、compiler driver dependency assembly、official std Package publication/bootstrap及
必要聚焦测试。不得修改runtime、deployment projection或consumer service。

## 必须实现

- 定义只读`ResolvedPackageSchema` compiler input view，按exact Package dependency binding携带已验证的
  `PackageSchemaIndex`和`PackageSchemaTypeRecord`；projection crate不得自行访问filesystem。
- compiler driver从canonical store解析每个实际可达Package requirement的schema，并绑定到exact dependency
  alias/package id/build；缺index/record、owner或ABI/build绑定不一致时fail closed。
- 当前Package自己的ServiceContract projection也必须能取得刚生成的resolved records，而不是只拿
  PackageArtifact refs；API应支持F161沿descriptor计算精确传递闭包。
- official `skiff.run/std`必须作为普通Package生成并存储其公开schema index/type records；至少覆盖当前
  schema-stable HTTP request/response/response-stream-event类型。consumer通过隐式exact std dependency取得
  同一bundle，不允许调用`canonical_http_boundary_type`伪造结构或另算身份。
- schema input只暴露`api.yml`显式公开类型；遇到boundary引用未公开named type时提供结构化fail-closed事实。
- 不恢复旧`ContractTypeId`、`PackagePublic`、ServiceContract `boundarySchema`或双轨兼容。

## 验证

- store-backed真实driver输入：普通dependency与implicit std均得到exact resolved schema。
- 缺schema record、错owner、错build/ABI binding、未公开named type均拒绝。
- std HTTP bundle的owner是`skiff.run/std`，且consumer侧没有第二份descriptor生产源。
- projection-input与driver聚焦测试、std bootstrap测试、`git diff --check`。
- 若F159硬切导致下游crate仍不能完整编译，必须用最小crate/fixture验证本任务接口，并记录精确断面；不得加兼容。
- 独立提交并写result。

