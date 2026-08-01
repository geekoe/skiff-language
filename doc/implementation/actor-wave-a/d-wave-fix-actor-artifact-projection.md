# D 波：Actor 声明投影进 PackageArtifact

## 目标

把 actor 声明投影进 `PackageArtifact`，使依赖方（尤其 `kind: test` 服务的
`topLevelAlias` 视图）能解析 actor 声明并用于 `std.actor.get` 类型参数。

## 依据链

- 权威依据：`skiff_blockers_preflight`（主 Agent 提供）与仓库
  `/Users/geek/workspace/skiff/AGENTS.md`。
- 流程：`/Users/geek/workspace/multi-agent-development.md`「开发 Agent」章节。
- 基线：`integration/actor-wave-a` HEAD `1532bd7b`（task 指定，已验证）。
- 父节点：skiff_blockers_preflight 定位的根因摘要（task payload）。

## 根因

- `PackageLocalAbiSymbol::Type` 没有 actor 形态/元数据。
- projection 从不消费 `FileIrUnit.actor_declarations`。
- `TypeResolutionModel::actor_type_resolution` 只查本地 `source_types`，
  `actual_receiver_symbol` 对 `PackageSymbol` 只放行 `Record`。
- 依赖方通过 `topLevelAlias` 使用 `std.actor.get<subjectImpl/thread_actor.ThreadActor>`
  时报 “type argument ... is not an actor declaration”；artifact 里 ThreadActor 只是
  `kind: record`。

## 选型

在 `PackageLocalAbiSymbol::Type` 上新增可选字段
`actor: Option<PackageActorAbi>`（不新增独立 `Actor` 变体）：

- actor 附属于同文件 type declaration，同一 source path 只能有一个符号；独立变体
  会造成同 path 双符号冲突，且要改写 validation、indexing、http gateway 等全部
  Type 消费面。
- `PackageActorAbi { actor_abi_identity, abi: ActorAbiInput }`：`ActorAbiInput`
  自带 wire 校验（key 字段存在、id 类型匹配、字段去重、method 约束），并覆盖
  task 要求的 key/create/public method/identity 元数据。
- `TypeExport` 同步增加可选 `actor` 字段，保证 symbol 与 implementation link
  一致（consumer indexing 与 artifact validation 都校验二者相等）。
- 类型引用在投影时按各自视图归一化：public export 用 visible 归一化，
  implementation 用 implementation 归一化。

## 写集

- `artifact-model/src/package_artifact.rs`、`artifact-model/src/package_unit.rs`、
  `artifact-model/src/schema.rs` + tests。
- `compiler/projection/src/package_artifact/export_links/mod.rs`（public link）、
  `compiler/projection/src/package_artifact/callables/mod.rs`（implementation
  symbol/link）+ 对应 tests/fixtures。
- `compiler/source/src/type_resolution_model.rs`、
  `compiler/source/src/type_resolution_model/query.rs` + tests。
- `artifact-identity/src/constants.rs`、`validation.rs` + tests。
- 编译波及的机械修改（仅补 `actor: None`）：`deployment/src/projection/tests/operation_bindings.rs`、
  `runtime/driver/eval/tests/support/program.rs`、
  `runtime/eval/src/actor_executor/tests/.../callback_matrix.rs`。
- 新增 compiler/tests consumer fixture（`compiler/tests/package_imports.rs` 或新文件）。

不在本波写集：compiler/lowering（A/B 波）、runtime/boundary+eval 实现（C 波）、
router/runtime 在途波、`doc/reference` 公共文档。

## 实现要点

1. artifact-model：`PackageActorAbi` + `Type.actor` + `TypeExport.actor`；
   `PACKAGE_ARTIFACT_SCHEMA_VERSION` v9→v10，v9 进入 retired 列表。
2. projection：
   - public：`project_package_export_links` 构造 `TypeExport` 时按 actor name
     查找 `unit.actor_declarations`，用 visible 归一化投影 `PackageActorAbi`。
   - implementation：`project_implementation_types` 对每个 type declaration 查找
     同名 actor，用 implementation 归一化投影到 symbol 与 link。
3. source：
   - `SourceTypeKind::Actor` 增加 canonical id_type/fields/create（TypeRefIr 形态），
     与 `Record.canonical_fields` 同模式；source 索引保持 canonical 为 None。
   - `index_artifact_package_types` 从 `Type.actor` 构建
     `SourceTypeKind::Actor`（is_alias/is_interface 必须 false）。
   - `artifact_symbolic_type_index` 校验 symbol.actor == link.actor。
   - `actor_type_resolution` 增加 package 分支：按 exact view（topLevelAlias /
     public alias）查 `package_types`，用 canonical 形态直接构造
     `ActorTypeResolution`；module_path/name 用
     `package_receiver_source_symbol_path` 还原 provider 内部 source path。
4. artifact-identity：Local ABI marker v5→v6、prefix v7→v8；Build marker v8→v9、
   prefix v10→v11；validation 校验 actor ABI 形状与 link 一致性。

## 自验收

- `cargo test`（artifact-model、artifact-identity、compiler/projection、
  compiler/source 聚焦 + 受影响 workspace 包）。
- 新增测试：
  1. projection actor fixture：package_local_abi 出现 actor 声明（不是普通 record）。
  2. compiler/tests consumer：`std.actor.get<subjectImpl/thread_actor.ThreadActor>`
     经 topLevelAlias 编译通过，FileIR 中 T0 为 ServiceSymbol、T1 为 id 类型。
  3. artifact-identity：篡改 key/create/method 后 ABI/build identity 变化。
- `node scripts/verify.mjs`（基线 36/36 保持全绿）。
- 记录需重跑的外部 acceptance：
  `node scripts/run-actor-full-chain-acceptance.mjs`（actor-full-chain）、
  `router/tests/compilerGeneratedManifestCompatibility.test.ts`（identity 变化后
  router fixture 需重新生成/核对）。
