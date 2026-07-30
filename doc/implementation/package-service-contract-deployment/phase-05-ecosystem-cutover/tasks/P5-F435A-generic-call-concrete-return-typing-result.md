# P5-F435A Generic call concrete-return expression typing result

状态：`IMPLEMENTATION_PASS`。真实 AIHub 已越过 F434A 冻结的三处 generic
`encodeJson<T>(...) -> Json` expression-type 诊断；canonical publish 随后暴露一个独立的
AIHub interface object-safety blocker。本文按任务边界记录新首错并停止，没有修改 Internals、
承接该 blocker 或运行后继 combined。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 冻结父输入 | `9391a895409c88b678c3c50a74a4dc83066540e9` | `318360137a460f2eda4ab00682a58a9438fe2371` |
| task dispatch 输入 | `9e2c3bb1e7a235632310037c5feba375719a1c9a` | `60f6f4dfa2524ac4ca934f620c6bdcee9523bb74` |
| implementation | `0f03cdc7c196aaeb202afd1ca78c100fc7dec277` | `ccadd9154549340851ffe0c422ffa175be382cbf` |

implementation 只修改：

- `compiler/source/src/expression_type_model.rs`
- `compiler/source/src/expression_type_model/object_materialization/tests.rs`

除此之外只新增本文 result。没有修改其它 compiler crate、runtime、router、test-runner、
skiff-packages、AIHub、Agine 或任何 Internals 文件。

## 2. 实现边界

`resolve_callable_signature_type` 继续优先使用既有 exact type-argument substitution 和
structured substitution。新增的 fallback 只覆盖两种原有/目标情况：

1. callable 本身没有 type parameter，保持既有普通声明类型解析；
2. generic callable 完全省略显式 type args 时，尝试按声明 context 解析 signature type。

第二种结果只有在完整 `TypeRefIr` 递归检查不含任何 unresolved `TypeParam` 时才保留。因此：

- `encodeJson<T>(value: T) -> Json` 的 call expression 获得真实 `Json` fact；
- `identity<T>(value: T) -> T` 不获得伪 concrete fact；
- `singleton<T>(value: T) -> Array<T>` 的嵌套 type parameter 同样保持未解析；
- 已提供显式 type args 的 exact/structured 路径及其 arity 行为不变。

本实现没有添加 generic inference、`any`、AIHub 名称/路径特例，也没有把 call expression加入
`expression_accepts_contextual_target`。因此 target-typed object literal 仍消费 call 的真实类型，
不会把 field target 冒充为 call 类型；missing、extra、incompatible field 与 nominal/package
identity 检查仍由原 owner 执行。

## 3. Direct tests

新增两个 direct tests：

- concrete-return 正例同时验证：
  - unannotated local binding initializer 的 generic call fact 是 `Json`；
  - 从该 initializer 推导出的 binding identifier fact 是 `Json`；
  - target-typed `JsonObject` field 中的 generic call fact 是 `Json`，且 materialization 成功。
- dependent-return 负例分别把 `T` 与 `Array<T>` call 放入 target-typed object field，继续得到
  `object literal field ... has no resolved expression type`。这也锁定 F240 的 unresolved
  non-identifier 负边界没有被扩大成 contextual target。

聚焦命令：

```text
cargo test -p skiff-compiler-source --lib generic_callable
```

结果：`3 passed, 0 failed`；其中 2 项为本 leaf 新增测试，另 1 项为名称匹配到的既有 generic
signature handoff 测试。

## 4. 验证

所有 Cargo 命令使用隔离的 `CARGO_TARGET_DIR`，未写入 worktree build 目录。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-source --lib` | PASS，323 passed、0 failed、0 ignored |
| `cargo check -p skiff-compiler-source` | PASS；只有仓库既有 unused/dead-code warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

## 5. 真实 AIHub crossing evidence

只读输入：

| Checkout | Commit | Tree | 状态 |
| --- | --- | --- | --- |
| `/Users/geek/workspace/internals-phase-05-integration` | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` | clean |

执行：

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f435a-generic-return \
  npm --prefix /Users/geek/workspace/internals-phase-05-integration/aihub/service \
  run type-check
```

canonical workflow 不再报告 F434A 的三处诊断：

```text
internal.aihub_service: return object literal field `event` has no resolved expression type at 2180:12
internal.provider_catalog: return object literal field `reasoningLevels` has no resolved expression type at 123:22
internal.provider_catalog: return object literal field `reasoning_levels` has no resolved expression type at 124:23
```

它继续进入 AIHub package typed File IR lowering，随后停止于新的独立首错：

```text
failed to lower package agine.ai/aihub source internal/aihub_service.skiff to typed File IR unit:
type `AihubManagedLlm` implements invalid interface selector `AihubManagedLlmClient`:
interface selector `AihubManagedLlmClient` is not object-safe:
method validateChat must declare `self: Self` as its first parameter;
method streamChat must declare `self: Self` as its first parameter;
method webSearch must declare `self: Self` as its first parameter
```

最小 owner 是只读 Internals 的
`aihub/service/internal/aihub_service.skiff:6-9`：interface 三个 method 声明没有 receiver；
同文件 `impl AihubManagedLlm` 的实现方法已有 receiver。该问题不属于 expression-type owner，
本文未修改它。按任务规则，完整 AIHub service test、identity comparison、AIHub combined 和
Agine combined 均未承接。

## 6. 隔离与禁令

- 未启动或修改 stable instance、watch registry、router、runtime、telemetry 或 MongoDB。
- 未运行 `build`、`dev`、`start`、reload、live provider 或 fixed-port workload。
- 未 merge、rebase 或 push。
- AIHub type-check 使用临时 ecosystem store；隔离 Cargo/NPM/TMP 产物在交付前删除。
- implementation 与 result 分开提交；result commit/tree 由交付消息记录。
