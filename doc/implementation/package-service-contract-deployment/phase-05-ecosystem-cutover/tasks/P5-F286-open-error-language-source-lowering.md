# P5-F286 Open error language, source and lowering consumers

状态：Ready。

## 直接父节点与权威链

- 直接父 checkpoint：
  `P5-F291-open-error-compiler-consumer-checkpoint-result.md`

父 checkpoint 已引用本节点所需的全部共享结果，并可追溯到唯一权威架构。启动时只读本任务；
需要确认共享接口或代码事实时才沿父链向上读取。

## DAG 位置、基线与解除关系

- 节点：F280 `W2-L Language and lowering` 中除已完成 F287 std 与 F290 effects 外的 consumer。
- exact base：
  `c08077c2efb6826f8f9dbd802f211fe9d4106115`
- 当前成熟度：实现检查点，不是稳定候选。
- 前置 F284/F285/F287/F288/F290 均已合入；不得修改其公共 DTO、identity、std surface 或 effect
  语义。
- 完成后解除：
  - W2 compiler/artifact combined probe；
  - A2 language 独立验收；
  - W2-R runtime identity/channel。

## Production owner 与写入范围

允许：

- `compiler/core/src/type_closure/**`
- `compiler/source/**`，但明确排除：
  - `compiler/source/src/callable_effects/**`
  - F287 已冻结的 prelude registry、`semantic/interface.rs` 与 std surface，除非只是由新
    declaration enum 导致的必要穷举适配；不得重新引入 marker/builtin
- `compiler/lowering/**`

允许迁移这些 owner 的 co-located tests，以及 `compiler/tests/**` 中只构造/断言本任务
declaration、throw/catch/rethrow、instruction site 或已删除 error-set DTO 的机械 fixture。

另精确允许 `compiler/core/src/spawn_targets.rs` 的 test module 中现有 `service_file_ir` 手写
`CallIr` fixture补 required synthetic site；只能使用
`CompilerGeneratedTestHarness`，不得修改该文件 production spawn projection。

禁止修改：

- syntax surface/grammar；
- artifact-model、artifact-identity、compiler compiled/projection-input/projection/contract/input、
  deployment 与 test-runner production；
- runtime、router、telemetry、std/prelude source、skiff-packages、internals；
- 权威 reference/architecture。

若 production 需要超出上述 owner 的新 helper 或公共 shape，立即返回设计/owner 缺口，不自行扩大。

## 完成标准

### 1. 唯一 source `CatchLeaves` 规则

在 source semantic/type model 中建立一套可复用的 catch-leaf 分析，statement throw、expression throw、
test-effect throw、catch 与 rethrow 共用，不能各自复制 shape 判断。

- 任意用户 `type` 声明均有名义 catch identity：
  - nominal record；
  - nominal representation，包括 primitive-backed representation；
  - named union 的 actual concrete/synthetic/literal branch identity。
- transparent `alias` 按 RHS 展开，不创建 identity。
- anonymous union 取 branch leaves 并集；每个可能 branch 都必须有确定 identity。
- 未包装的 primitive、literal、anonymous record、container、interface、`unknown`、function、
  nullable/null branch和无约束 type parameter均非法。
- fully-instantiated generic nominal type合法；未实例化或无法确定 runtime identity 时 fail closed。
- alias/recursive traversal 使用既有 canonical type owner 与 cycle policy，不能按短名、display 或 shape
  猜测。

静态规则：

- `throw expr`：静态类型的每个可能运行时值都有 identity；
- `catch<E>`：`CatchLeaves(E)` 有效且非空；
- `rethrow expr`：操作数必须是 `Exception<E>`，且 E 的 leaves 有效非空；
- rethrow 保留现有 exception，不创建新的 throw site。

这些检查只判断名义 catch identity；不得要求 `PublicNameable`、`SchemaClosed`、可序列化或
`ErrorPayload`。

### 2. 删除 source closed error-set 假设

- `SourceExecutableSignature::package_callable_signature` 只产生 parameters、return、maySuspend。
- source contract call typing 不读取 `operation.errors`，不拒绝开放错误通道。
- test-effect `throw:` 接受任意满足同一语言 throw 规则的名义值，不读取 package/service declared
  throw types。
- 保留 throw `payload_type` 与 throw provenance；不新增函数签名 `throws`、推导集合或 compatibility。
- F290 的 service-call `Fresh` throw origin及所有 detached guarantees保持不变。

### 3. declaration 与 named-union branch lowering

所有 source/prelude declaration 必须精确产出 A1 冻结的互斥 `TypeDescriptorIr`：

- nominal record → `Record`
- nominal representation → `Representation`
- named union → `Union`
- transparent alias → `Alias`
- interface → `Interface`

named union 的每个 branch 必须确定地产出：

- concrete nominal branch及完全实例化 type arguments；
- anonymous discriminator record的 payload type、discriminator field/value；
- literal branch value。

enclosing union context由所属 `TypeDeclIr`保留。相同 shape/tag 位于不同 named union 时不能共享
branch identity 输入；anonymous union本身不创建 identity。不得把 interface降为空 record，或把
representation与alias重新合并。

所有 type-closure、assignability、external/publication ref traversal必须对五种 declaration和三种
branch穷举处理，保持 F285 package owner/type-slot与 exact `PackageSchema` facts。

### 4. required source/synthetic instruction site

- source-authored statement/expression throw与所有 source-authored call写入真实 `SourceSpanRef`。
- compiler-generated wrapper、desugaring与test harness只能使用 A1 有限
  `SyntheticInstructionSiteReason`中的准确原因，不能伪造 source path/span。
- transform/rewrite pass必须保留已有 site，不能重建或丢失。
- `ExprIr::Catch.catch_type`始终写 required type；删除 `Some`/`None`与隐式 catch-all。
- rethrow不新增 site。
- 相同源码的 File IR/source map输出必须确定；注释或无关声明不能改变已有 instruction 的错误位置。

### 5. 诊断与非回归

- 所有非法 leaf在 compile/source phase给出定位到 throw/catch/rethrow 的诊断，不能推迟为 runtime
  Decode。
- wrong dependency parameter/field、private slot 与 exact projection继续按 F285 fail closed。
- 不改变 ordinary return typing、service call target、expected Local ABI、same-heap eligibility或
  source-visible语法。
- 反向搜索在本任务 production owner中必须清零：
  `BoundaryErrorContract`、`contract.errors`、`throw_types`、optional catch construction、旧 union
  `variants` field和无 site `CallIr`/throw constructor。

## 最小测试矩阵

必须增加真实 source→File IR tests，至少覆盖：

1. record、primitive-backed representation、transparent alias；
2. named union的 concrete、anonymous discriminator、literal branch；
3. anonymous union的两个 nominal leaves；
4. fully-instantiated generic nominal；
5. 同 shape不同 nominal、同 tag不同 enclosing union；
6. primitive、literal、anonymous record、container、interface、unknown、function、nullable、
   unconstrained generic与mixed union的 throw/catch负例；
7. 非 `Exception<E>`、非法 E 与合法同-envelope rethrow；
8. package-direct与service test-effect throw不读取 declared set；
9. source throw/call span、synthetic wrapper reason、required catch type及 rewrite 后 site保留；
10. F285 dependency callable result field-read回归仍存在。

唯一验证 owner：

```bash
cargo test -p skiff-compiler-core --lib -- --list
cargo test -p skiff-compiler-core --lib --no-fail-fast
cargo test -p skiff-compiler-source --lib -- --list
cargo test -p skiff-compiler-source --lib --no-fail-fast
cargo test -p skiff-compiler-lowering --lib -- --list
cargo test -p skiff-compiler-lowering --lib --no-fail-fast
cargo test -p skiff-compiler --test package_imports -- --list
cargo test -p skiff-compiler --test package_imports --no-fail-fast
git diff --check
```

先确认 selector 非零。若 `compiler/tests/**` 的无关机械 fixture仍遮挡某条集成测试，记录精确文件并只在本任务
已授权的 test-only范围迁移；不得修改别的 production owner。不得运行 workspace、完整 compiler、
生态发布、stable、live或chat smoke。

## 风险、验收与交付

- 风险：高；验收组：`A2-language`，实现合入后必须由新的只读 Agent独立验收。
- 最早风险探针：core/source/lowering 三 crate首次共同编译后，先跑非法 leaf负例与 File IR site/branch
  golden，再跑各 owner完整 lib suite。
- worktree：`/Users/geek/workspace/skiff-p5-f286-error-language`
- branch：`codex/p5-f286-error-language`
- 从 exact base 建立，不 push、不操作 stable。
- 启动到第一次 production 修改不超过5分钟；不可执行时立即返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 完成后提交并返回 commit、变更摘要、反向搜索、自验收矩阵、未执行/被遮挡测试与任何设计缺口；
  不得自行承接 runtime、combined gate或验收任务。
