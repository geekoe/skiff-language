# P1-T09：实现 Package Boundary Projector

状态：`ready`
类型：Compiler projection / Publication ABI
依赖：P1-T03、P1-T06、P1-T07、P1-T08
执行者：Boundary Projector Agent，一份提交

## 目标

为每个 package public callable 生成显式 boundary projection：可用时生成完整即时
`LinkableValuePlan`/operation contract，不可用时生成稳定 reason codes。Local Code ABI 始终保留，
因此本任务不能通过禁止 mutable helper 来让 projection 变简单。

## 单一 Owner

在 T06/T07 拆出的模块中建立一个 package boundary projector，例如：

```text
compiler/projection/src/boundary/
  mod.rs
  callable.rs
  value.rs
  callback.rs
  stream.rs
  errors.rs
  availability.rs
```

`compiler/publication-abi` 可负责 canonical signature/operation builder，但 boundary eligibility 和
value plan 只能由上述 projector 决定。service projection、linker、runtime 不得再推导一次。

## Projection 规则

### Ordinary data

- primitive/record/array/map 等生成 detached boundary graph plan。
- 初始实现可保守拒绝依赖 caller alias identity、cycle 或无法证明 closure 的类型。
- in-process binding 未来也必须消费同一 plan，不能把引用直传当成额外能力。

### `any I` 与 native handle

- 只生成 request-scope callback capability：owner、operation projection、route kind、lifetime、
  cancel/invalid behavior 都是 contract 的一部分。
- 不携带 method table、concrete native object 或 runtime-local handle。
- 缺 method/native callback adapter 时 `Unavailable`，不能退化成 opaque JSON。

### Stream / error / timeout / cancel

- 使用已有 typed contract 基础并纳入 operation/boundary identity。
- value plan只投影T01冻结的owner、operation、request lifetime、失效和item/error/cancel channel。
- callback重入调度、stream buffer/backpressure、non-cooperative cancel enforcement属于Phase 03
  Execution Contract，不由projector推导，也不进入Phase 01 boundary identity。

### Recoverable overlay

- ordinary service parameters/returns只做 linkable projection。
- 只有显式 durable lane 才调用 T07 的 recoverable projector。
- request-scope callback capability 默认在 recoverable overlay fail closed。

## Availability

每个 public callable必须输出：

```text
Available(BoundaryOperationContract)
Unavailable([BoundaryUnavailableCode...])
```

至少覆盖 caller-reachable mutation、returned/escaped alias、same-heap requirement、unknown effect、
unsupported type closure、missing callback/native adapter。reason code 排序 deterministic；人类
detail 可含 source span/call chain，但不进入 identity。

## 范围

- T06 package projection 的 boundary extension point
- T07 拆出的 shared type index 与 publication ABI builder
- artifact-model T03 contract 的构造/校验
- compiler projection/publication-abi tests

## 非目标

- 不解析 package service requirements。
- 不创建 caller stub package 或 runtime adapter code。
- 不做 ServiceUnit operation selection；Phase 02 负责。
- 不实现 remote wire format。
- 不执行 boundary call。

## 必须测试

- pure ordinary-data callable 为 `Available`。
- parameter mutation、return alias、same-heap identity、unknown callee 为结构化 `Unavailable`，但
  Local Code ABI 仍存在。
- `any I`/native handle 只有 callback capability plan；adapter 缺失失败。
- stream/error/cancel/timeout 改动影响 boundary identity。
- diagnostic wording/source path 改动不影响 identity。
- ordinary boundary 不触发 recoverable validation；显式 durable lane 会触发。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-projection boundary
cargo test --no-fail-fast -p skiff-compiler-publication-abi
cargo test --no-fail-fast -p skiff-artifact-model -p skiff-artifact-identity boundary
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

## 验收标准

- 所有 public callable 都有可解释状态，无 `Option::None`/字段缺失猜测。
- mutable local helper 未被语言或 package API 禁止。
- linkable 与 recoverable projector 分层且共享基础不复制 policy。
- service/package 代码路径不维护第二套 eligibility。

## 停止条件

- effect summary 不足以 sound 地判断 caller-visible alias/mutation；
- callback capability lifetime/owner/operation surface 未被 T01 唯一确定；
- 必须读取 deployment config 才能生成 code boundary contract；
- 必须把 provider package/build identity写入 boundary operation identity。

## 提交

提交信息建议：`feat(compiler-projection): project package boundary contracts`
