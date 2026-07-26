# P5-F399 Canonical FileIR builtin spelling result

状态：Complete。

## 结果

compiler 现在只有一份可枚举的 source spelling → canonical FileIR builtin registry。合法 source
alias 仍可被语言接受，但所有 compiler 生成的 `TypeRefIr::Builtin.name` 在 artifact boundary 前
已经归一：

- primitive `boolean` 与 `bool` 都生成 `bool`；
- 18 个 compiler builtin 的 bare name 与 owner-qualified symbol 都生成各自的 canonical bare
  name，并保留 exact arity/kind；
- unknown、`String`、`Bytes` 与未声明的 `std.date.Date` 不会被隐式提升成 FileIR builtin；
- generic、nullable、union、record 与 function 中的 nested refs 使用相同的递归 canonicalization。

`compiler/source/src/type_resolution_model.rs` 与
`compiler/lowering/src/type_lowering.rs` 已删除各自重复的 builtin/qualified alias 表，统一消费
`compiler/core/src/prelude_registry.rs` 的 owner。Runtime linker production comparison 未改，仍执行
exact name/arity/schema identity 检查。

## Production 与测试覆盖

变更范围：

- `compiler/core/src/prelude_registry.rs`
  - 增加 typed primitive registry；
  - 增加可枚举的 `file_ir_builtin_source_spellings` 与 exact canonical lookup；
  - registry 测试验证 source spelling 无碰撞、18 个 builtin canonical name 唯一、alias 不改变
    arity/kind、unknown 返回 `None`。
- `compiler/source/src/type_resolution_model.rs`
  - 三条 source type resolution 路径只消费 core canonical lookup；
  - 删除 `builtin_type_name` 与 `canonical_native_prelude_type_symbol`；
  - 增加全 registry source resolution 与 undeclared alias 负例。
- `compiler/lowering/src/type_lowering.rs`
  - 所有本文件 AST/text → `TypeRefIr::Builtin` 构造集中到同一 canonical helper；
  - 直接 lowering 测试覆盖 15 个 qualified alias，以及 generic、nullable、union、record、function
    nested refs。
- `compiler/tests/builtin_canonical_spelling.rs`
  - 递归扫描 fresh std、ordinary package 的 FileIR、Package Artifact、Local ABI 与 PackageSchema；
  - 验证 `ConflictError.retryable` 四层均为 `bool`，schema identity 保持，producer identities
    按预期改变。
- `runtime/linker/src/assembly_execution/service_error_index.rs`
  - 仅增加 `cfg(test)` strict negative；canonical `bool == bool` 通过，人工
    `boolean != bool` 继续被 exact linker 拒绝。

以下要求的命令全部通过：

```text
cargo test -p skiff-compiler-core prelude_registry
cargo test -p skiff-compiler-source prelude_registry
cargo test -p skiff-compiler --test builtin_canonical_spelling
cargo test -p skiff-compiler --test prelude_std_schema prelude_builtin_schema_is_typed_in_file_ir
cargo test -p skiff-compiler --test prelude_std_schema builtin_types_reach_the_package_boundary_projection
cargo test -p skiff-runtime-linker service_error_index
cargo test -p skiff-test-runner --test canonical_std_seed_bootstrap -- --test-threads=1
cargo fmt --all -- --check
git diff --check
```

结果分别为 core 7 tests、source 22 tests、canonical spelling 2 tests、两个 schema selector 各
1 test、linker selector 5 tests、bootstrap 1 test；均为 PASS。

## Fresh artifact receipt

fresh artifact root 为 `/tmp/skiff-p5-f399-fresh.SqdFzq`，environment 为 `p5-f399`。使用当前
worktree producer 重新 bootstrap std，并构建
`test-runner/fixtures/package-service-host/helper`。记录 receipt 后，该隔离临时目录已移入废纸篓。

fresh std identities：

- Package Build：
  `skiff-package-build-v8:sha256:876f5091d949db40a397918e08107d406d3aea245d118fd8ae2e71a03eac052b`
- Package Local ABI：
  `skiff-package-local-abi-v6:sha256:d541c49401dc619cb2ab07300f4b610133a8111f3525ad5e10dd4ac6e200defd`
- std.db FileIR：
  `skiff-file-ir-v8:sha256:e62485ea5dcd42c0e4552db0e4271bc8bd573ca7478a09bfa238bd2183976cf8`
- PackageSchema index：
  `skiff-package-schema-index-v1:sha256:1f70d5626cddaab23d51d52db974a9292cf019cb0161d67ff560c599ed6fd7fe`
- `std.db.ConflictError` PackageSchema type：
  `skiff-package-schema-type-v1:sha256:dd893e08035a093080419ff2c04beda67c1dab2e95ddcc23dec12f9ce6d8bdd0`
- bootstrap assembly：
  `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f`

相对 P5-F398 baseline：

- PackageSchema index 与 `ConflictError` schema type identity bit-identical；
- std.db FileIR 从
  `skiff-file-ir-v8:sha256:bb39d35baa25cbfb50a1d146e21a18a2ad088940d34304b877e13e348543b069`
  改变；
- Package Local ABI 与 Package Build 按 producer output 改变；
- representative helper 的 build、Local ABI 与 schema index identity 均保持 P5-F398 baseline。

递归扫描 fresh root 中所有 JSON subtree 的 `{kind:"builtin", name:...}`，observed canonical
names/counts 为：

```text
Array×78 Json×24 JsonObject×15 Stream×13 bool×15 bytes×59 integer×81
number×4 std.websocket.WebSocketConnection×4 string×526 void×52
```

registry 中 16 个 non-identity source alias spellings（`boolean` 加 15 个 qualified symbols）观察
数均为 0。`ConflictError.retryable` 的 FileIR、linked type、Local ABI public/implementation 与
PackageSchema 全部为 `bool`。

## Isolated activation

安装当前 worktree Router 的 locked local dependencies 后，通过
`runInIsolatedTestRuntime` 启动独立 MongoDB、Router 与 Runtime，environment 为
`p5-f399-activation`。owner 的 readiness gate 验证 active assembly 的 environment、generation、
assembly identity、healthy replica 与 capability connection 全部与 fresh bootstrap receipt
一致；callback receipt：

```json
{
  "gate": "ServiceErrorTypeIndex passed",
  "generation": 0,
  "controlReady": true,
  "replicas": 1
}
```

因此 isolated std activation 已越过 `ServiceErrorTypeIndex` gate，随后 owner 正常停止所有独立
进程并释放端口。没有访问或修改 stable/live。

额外运行完整 `node scripts/run-skiff-tests.mjs` 时，同样先越过 Router/Runtime readiness、
assembly activation 并进入 `std` fixture；其后在既有的独立 migration guard
`package-test ingress is not yet migrated to deployment gateway entries` 停止。该 guard 不属于
本任务 acceptance，也不涉及 builtin spelling。

## 严格边界

未修改 `std/db.skiff`、PackageSchema/contract normalization、`artifact-model::TypeRefIr`、
Runtime linker production exact comparison或任何 reader compatibility。未 merge、rebase、push，
未操作 stable/live。
