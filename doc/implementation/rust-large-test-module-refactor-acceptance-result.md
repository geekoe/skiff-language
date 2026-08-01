# Rust 大型测试模块重构：独立验收结果

日期：2026-08-01

状态：FAIL（第一次 F verdict）

验收对象是稳定候选 E：commit `0a94d75b3d916e87ff2b0c3ea32bebcbda4fc4fe`，tree
`2f98ed10a01a4f6d8b945d34b9af5d885e797d88`。执行拓扑见
[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md)，唯一权威设计为
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)。本文件只记录 F 对该精确 E 的只读
verdict，不修改候选、设计、源码或既有 result。

## Blocking findings

### 1. Service DB 的三个测试 owner 与权威职责不一致

权威设计按行为 owner 划分领域，不以当前文件位置或 result 自述为准。E 上有三项错配：

| 测试函数 | E 上位置 | 权威 owner | 结论 |
| --- | --- | --- | --- |
| `service_db_capability_context_does_not_require_request_frame` | `runtime/service-db/src/tests/error_contract.rs` | `provider.rs` | capability context 属于 provider/capability 领域，不是 error wire/catch 合同 |
| `object_metadata_accepts_retention_field` | `runtime/service-db/src/tests/runtime_config.rs` | `metadata.rs` | object metadata 字段接受与校验属于 metadata 领域，不是 runtime publication/storage/client 配置 |
| `skiff_file_record_document_preserves_capability_record_fields` | `runtime/service-db/src/tests/metadata.rs` | `mapping.rs` | file record document 的字段保真属于 document mapping 领域，不是 metadata plan |

Git-object 预检在 E 上分别定位到上述三个文件，函数名均唯一。修复必须只做 owner 间机械移动，保持函数名、
测试属性和函数体，并维持原 102 个根测试的一一映射；不得借机改变测试或生产语义。

### 2. 权威 workspace rustfmt 完成标准未满足

`cargo fmt --all -- --check` 在 E 上失败，报告未由本阶段修改的
`runtime/linker/src/assembly/tests/cross_package_actor.rs`。该文件在权威设计的初始 baseline
`2b584edc6c30cec4acf8fd2c9e3bde85790fe234` 与 E 上的 Git blob 都是
`8bbe074b19c47f6178298a5a59950e876baeea86`，因此这是可精确归因的 baseline 既有失败，不是候选引入的
regression。

但是权威设计第 4 条完成标准明确要求 rustfmt / rust-quality 通过，且没有 baseline 豁免，所以该失败在严格
authority 下仍然 blocking。修复会触碰本阶段授权写集之外的无关 linker 测试文件；在用户明确授权前，任何
Agent 都不得擅自格式化或提交它。

## 其余静态验收

除上述 blocker 外，F 的静态项目为 PASS：

- 两个根 `tests.rs` 只声明设计规定的子模块；support 模块不拥有测试。
- callable-effects 六个领域、单一 `AnalysisFixture`、旧分析入口清理与显式 fixture 边界满足设计。
- Service DB 的通用/recoverable support 分责、单一线程安全 hooks trait implementation 与
  `prepared_runtime` 独立边界满足设计。
- callable-effects 86 个、Service DB 原根 102 个和既有 prepared-runtime 11 个测试的身份集合保持；唯一 live
  Mongo 测试的名称、`#[tokio::test]`、ignore 原因和唯一性保持。
- 生产文件、manifest/lockfile、schema、配置、依赖和 live/default 边界未被本阶段 diff 改动。
- line gate 仍是无 exception 的真实最大值 4073；两个重构领域不再是最大文件。

F 没有运行 focused tests、compiler/runtime selector、runtime/router/Mongo 或其它动态命令；动态 gate 证据由 G
唯一拥有，不能用以上静态 PASS 替代。

## Non-blocking documentation findings

以下只记录为 non-blocking，不纳入本次 repair batch，也不授权修改既有文档：

- `rust-large-test-module-refactor-service-db-result.md` 的行数陈述存在文档级问题；不影响候选源码行为。
- `rust-large-test-module-refactor-service-db-leaf.md` 仍标记 `in progress`，与已交付状态不一致。
- `rust-large-test-module-refactor-callable-leaf.md` 把真实入口写成不存在的
  `compiler/source/src/callable_effects/mod.rs`；实际声明入口是
  `compiler/source/src/callable_effects.rs` 中的 `#[cfg(test)] mod tests;`。

这些问题不得成为无限文档清理 gate。F 的最终 verdict 为 **FAIL**，只由两个 blocking finding 组决定。
