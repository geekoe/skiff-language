# P5-F417 Suspension compiler inference and projection

状态：Ready（N1）。

## 直接父节点

- `P5-F416-suspension-schema-identity-current-checkpoint-result.md`

若需要核对 current owner、终态推断矩阵或本节点与后继的关系，再沿父节点引用读取
`P5-D93-suspension-current-base-reconciliation-audit-result.md`。不要默认重新解释顶层设计。

## 精确起点与任务边界

- integrated N0 checkpoint：
  `c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60`；
- N0 implementation：
  `57d0a5551aaa62e5a71655050478c1447f94324d`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明上述三个 commit 都是 HEAD ancestor。当前是实现检查点，下游尚未合流；不把暂时的
workspace 断链当成稳定候选失败。

独占 production 写入范围：

```text
compiler/**
本任务 result
```

禁止修改 artifact-model、artifact-identity、deployment、runtime、router、scripts、test-runner、
cross-system fixture、ecosystem source或设计。不得恢复 N0 已删除的字段、增加兼容读取、修改 generation，
也不得派子 Agent。

## 必须实现的终态

### 1. 删除旧 requirement / protocol 摘要的 compiler consumer

- 删除 interface method、callback descriptor、service contract operation 上已不存在的
  `may_suspend` / cancellation producer、copy、fixture与断言。
- contract projection只生成 code-free boundary shape；不能从 provider concrete implementation
  复制 suspension bit。
- callback / schema / contract wire不得重新引入 provider effect bit。
- interface conformance只比较 receiver、参数、返回值和 flags shape；同一 requirement 的 concrete
  `may_suspend=false` 与 `true` 都合法。
- public concrete signature仍必须与 exact implementation link 的 `may_suspend` 一致。

### 2. 唯一 call-target / suspension fixed point

source-internal resolved target新增明确的 `InterfaceMethod` target，至少携带 interface、method ABI id和
slot；不要继续把已解析的 interface method降成普通 `Unknown(UnsupportedDynamicDispatch)`。
public `CallableTargetFact` 不要求新增 wire kind；无法表达时仍可投影为 Unknown。

caller `may_suspend` 的唯一矩阵：

| target | 结果 |
| --- | --- |
| local function / local impl / actor method | SCC exact；缺失 fact 时 `true` |
| native function / receiver builtin | registry exact；缺失 fact 时 `true` |
| config intrinsic | `false` |
| dependency Package callable | `exact_signature.may_suspend`；缺失 signature 时 `true` |
| source-internal interface method | `true` |
| service contract operation | `true` |
| unknown / unresolved | `true` |

`detached_contract_callee` 及 test-effect service target不得再伪造或读取 contract provider bit；
interface、service与unknown的保守 `true` 只是 caller 分析结果，不能生成 synthetic yield 或强制运行时
至少挂起一次。

### 3. 必须保留的 concrete facts与 F415 mapping

保留并精确传播：

- source executable、FileIR executable与 callable may-effects；
- Package public callable `may_suspend`；
- local / impl / actor / native / builtin concrete summaries；
- semantic facts、complete effects与 provenance；
- concrete public handoff。

以下 F415 路径的 `collection_name_mapping` ingest、validation与 exact clone必须保持：

```text
compiler/input-model/src/dependencies.rs
compiler/driver/generated_deployment.rs
compiler/driver/pipeline/mod.rs
compiler/projection-input/src/lib.rs
```

fresh requirement 的 non-empty mapping必须逐跳相同，不得用 empty fallback、default model字段或删除
validator来修复 fixture。

### 4. Current generation

compiler所有正向 producer / fixture必须只生成 N0 terminal current：

```text
PackageArtifact v9
canonical Local ABI prefix v7
canonical build prefix v10
PackageSchemaType v2
ServiceContract v5
ServiceProtocol v5
```

FileIR v8与 Publication ABI v1保持；不得加入 dual-read / dual-write。

## 验收矩阵

正例至少证明：

- 同一 interface requirement 的 concrete false / true 都 conform；
- dependency exact false / true分别传播为 caller false / true；
- interface / service / unknown始终保守为 true；
- concrete public handoff保持 exact；
- callback / schema / contract wire没有 provider bit；
- conservative true没有 synthetic runtime yield；
- non-empty collection mapping从 dependency到 requirement保持 exact。

负例至少证明：

- dependency exact signature缺失时 fail closed为 true；
- public concrete summary与implementation link不等仍拒绝；
- test-effect不能伪造 contract suspension bit；
- 含旧 contract字段的 fixture反序列化失败；
- shape、complete effects或 provenance的既有负例未弱化。

## 验证与交付

先用相同 selector加 `-- --list` 记录实际数量，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-core package_interface
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-source callable_effects
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-lowering suspend
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-projection package_artifact
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-compiled --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-contract --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test service_conformance
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked -p skiff-compiler
cargo fmt --all -- --check
git diff --check
```

D93 accepted listing基线依次为 `5 / 83 / 1 / 62 / 5 / 6 / 14 / 2`；以当前实际 listing为准，并解释
合理变化。不要运行 workspace/full isolated/stable/live。

写 `P5-F417-suspension-compiler-inference-projection-result.md`，记录 exact commit/tree、target/effect
矩阵、false/true artifact证据、contract/FileIR wire、mapping链、实际测试计数和所有未运行项。提交并
保持 clean；不 merge/rebase/push。

若一次有界探查后发现必须越过授权 production root、公共契约仍不明确或任务实际拆成多个新 owner，停止并返回
`TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`，不要自行扩大范围。
