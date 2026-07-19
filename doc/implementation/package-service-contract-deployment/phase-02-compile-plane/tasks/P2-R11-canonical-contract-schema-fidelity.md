# P2-R11：Canonical Contract Schema Fidelity

状态：port；只移植已独立验收的单一 commit `834cd55` 到 clean checkpoint。不得引入其 parent integration
history；若发生冲突，只按本任务 canonical schema owner 解决并重验受影响面。

## 背景

R10 迁移旧 prelude/service conformance tests 时发现，Phase 02 typed `ServiceContract`
只能保存 union variant list，不能表达语言已有的 discriminator/branch tag 语义；contract
validator 也只检查引用闭合，会接受任意 builtin 拼写/arity、非法 map key 和用户递归类型。
这些事实影响 remote wire 解释和 `ServiceProtocolIdentity`，不能交给未来 JSON renderer
补齐。

## 权威语义

除总设计外，本任务必须遵守 `doc/reference/static-semantics.md` 中已有语言事实：

- 命名 union 保留 enclosing union identity 与 branch identity；anonymous record branch 的 identity
  由 union type id 和 discriminator literal 派生。
- `Map<K,V>` 的 key 只能是精确 `string` 或单一名义 representation over string，名义
  identity 不得退化为普通 string。
- 当前用户 recursive alias/type 非法，recursive record 暂不进入 service boundary；
  compiler-known `Json`/`JsonObject` 是显式例外。
- `T?` 与规范化后的 `T | null` 是同一 nullable 语义。

## 实现要求

1. canonical contract schema 显式区分 structural union 与 discriminated named union。后者保存
   非空 discriminator field 与稳定、唯一的 branch tag -> branch type 映射；顺序不影响 identity，
   field/tag/branch 变化必须改变 protocol identity。
2. 补齐 contract literal/representation 的最小 typed 语法，使 discriminator branch 和单一
   nominal representation-over-string map key 可精确表达；transparent alias 不得冒充
   nominal representation。
3. definition compiler 在 identity 派生前做唯一 semantic normalization；已物化
   `ServiceContract` validator 只接受规范形，不在 read/validate 时静默重写。至少规范化
   nullable union、builtin canonical spelling/arity 与可安全展平的 structural union。
4. builtin grammar fail closed；`Map` 执行 key/value arity 和 map-key identity 规则。不允许
   arbitrary string 成为新 native/builtin wire 协议。
5. schema graph validator 按当前政策拒绝 alias/union cycle 和用户 recursive record，但不把
   compiler-known `Json`/`JsonObject` 当作未闭合的用户引用。
6. schema/wire/identity marker 与 golden 随 canonical shape 一次性更新；Skiff 未发布，不保留
   dual-read、field alias 或旧 identity fallback。

## 非目标

- 不生成 JSON Schema、`oneOf`、`xSkiffUnionDiscriminator`、`$defs` 或 serviceAssembly
  presentation；它们未来只能从 canonical typed schema 单向派生。
- 不新增 contract YAML/IDL/CLI authoring，不从 provider source 反推 contract。
- 不改 PackageArtifact code identity、effect/lowering、ingress 或 runtime；旧 compiler service 路径由
  T05 在 clean base 上终态替换。
- 不提前开放 guarded recursive record；未来开放时另行升级 canonical schema。

## 允许写入

- `artifact-model` 的 contract schema typed model/schema version 及直接 strict-wire tests。
- `artifact-identity` 的 contract normalization validation、identity projection/golden/mutation tests。
- `compiler/contract` 的 definition normalization/diagnostics 及直接 tests。
- 仅在适配新 canonical 拼写所必需时，修改 package boundary type projection 与直接 tests。

禁止修改 driver/service publication、source/lowering/compiled、runtime/router 或 R10
integration fixtures。发现必须跨越边界时停止并回报。

## 完成态

1. code-free definition 可产生 discriminated union 与合法 map schema，round-trip/strict wire 通过。
2. discriminator field/tag/branch、map nominal key identity 和 representation target 改变均改变
   `ServiceProtocolIdentity`；map 顺序不影响 identity。
3. duplicate/missing discriminator tag、非法 builtin/arity/map key、dangling ref、alias/union/record recursion
   全部 fail closed，诊断指向精确 schema stable key/path。
4. `T?` 与 `T | null` 经 definition normalization 得到同一 canonical contract 和 protocol identity；
   loaded artifact 的非规范 raw shape 被拒绝。
5. contract/artifact-identity/compiler-contract 聚焦 tests、targeted rustfmt、`git diff --check` 通过，
   commit 且 worktree clean。完整 foundation/compiler gate 仍由 T07 合流后执行一次。
