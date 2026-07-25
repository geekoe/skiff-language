# P5-F273：Public Alias Expansion Result

状态：COMPLETE。实现基于 `3c929affd93d15d60458fd154e24c78b854a47b0`。

## 实现结果

- `alias` 现在在 source type resolution 与 File IR 最终化阶段递归展开为精确 RHS；builtin、字面量联合、
  nullable、record、container、callback、any-interface 以及外部 package type ref 均保留结构化类型。
- `PackageLocalAbi` 明确记录 transparent alias 分类。alias 仍携带可验证的 RHS descriptor，但不再获得
  `PackageSchema` nominal identity；record、representation、actor、interface 等 nominal declaration 保持不变。
- package artifact ingest 保存 canonical alias RHS 与 canonical record field IR，不再把 artifact 类型先格式化为
  source text 再反解析。该修复同时避免 `Nullable(Union)` 被错误解析成 `Union(..., Nullable(...))`。
- executable signature、operation contract、usage descriptor、schema closure 和 contract-aware type resolution
  均消费展开后的结构化 IR。cycle、缺失 RHS、非自描述 local index 和无法唯一解析的 public reference
  继续 fail closed。

## 验收证据

- 新鲜 A → B → C package artifact 测试覆盖 scalar alias、string literal union、nested nullable container、
  cross-package nominal RHS；B 的 alias 无 schema identity，A 的真实 record 仍为 nominal。
- source/artifact 单测覆盖 callback、nested structural type、representation、missing RHS、recursive cycle，
  并覆盖 artifact record field 的 exact `Nullable(Union)`。
- 新鲜隔离发布依次成功完成 std、http-session、track、llm-api、llm-providers、Agent、codex-relay 和 AIHub。
  Agine consumer 已越过 `canonical.SubagentExecutionStatus` 及随后发现的 `LlmApiFormat` alias 位置；
  当前继续停在独立的 nullable narrowing、iterable 与 object-literal target 问题，不再出现本任务的 alias
  nominal mismatch。

验证命令：

```bash
cargo test -p skiff-artifact-model
cargo test -p skiff-compiler-source -p skiff-compiler-lowering -p skiff-compiler-projection
cargo test -p skiff-compiler --lib
cargo test -p skiff-compiler --test package_imports
cargo check --workspace
```

以上全部 PASS：artifact-model 134、source 290、lowering 43、projection 33、compiler lib 19、
package imports 10。

隔离探针使用 canonical `withCanonicalAssembly` 流程，并补入
`skiff-packages-phase-05-integration/http-session` 与 `track` 两个 package roots。现有
`check-isolated-service-graph.mjs` fixture 尚未列出这两个 Agine 直接依赖，直接运行会在到达 Agine
类型检查前报告 missing PackageArtifact pointer。

## 非本任务基线

`cargo test -p skiff-compiler --test file_ir_execution_type_representation` 的两个测试仍因本任务基线已有的
self `PackageId` rewrite 问题失败；该行为来自基线祖先 `fc347441`，本任务未修改对应 emission 路径。
