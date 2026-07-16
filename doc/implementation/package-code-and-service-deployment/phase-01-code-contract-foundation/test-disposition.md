# Phase 01 测试处置表

状态：`ready`。本表描述测试语义如何迁移，不要求每个任务跑全量。

## 1. 原则

- 按行为处置，不按文件或测试数量机械迁移。
- 旧 service source path 在 Phase 01 仍存在，因此相关编译测试默认 `keep`；只改为消费共同
  typed fact 的部分才 `rewrite`。
- 只验证“package 不能声明 service dependency”或“effect 永远为空”的测试应删除，并用新
  契约测试替代。
- artifact schema 已改变就直接更新 fixture；不添加 legacy reader。
- 拆分任务只移动测试并证明行为等价，不趁机扩大 assertion surface。

## 2. Keep

以下语义不因本阶段改变：

- parser/name/type resolution、package import visibility、package dependency transitive resolution；
- package local call 的 heap/alias/mutation 语义；
- 当前 service source compile、ingress、DB、spawn、actor、recoverable projection 的既有行为；
- PackageUnit dependency ordering、implementation links、file identity、test-only overlay isolation；
- service artifact-root 的 fail-closed 读取和 schema validation 基础；
- compiler crate boundary/DAG、artifact identity single-owner 脚本。

现有测试若只因 DTO 新增必填字段而失败，应由 fixture builder 提供显式默认值；不应在生产
deserialize 上使用兼容默认隐藏缺失字段。

## 3. Rewrite

| 当前行为域 | 新断言 |
| --- | --- |
| package manifest validation | 顶层 `services` 合法；未知字段、重复 alias、非精确 version 仍 fail closed |
| service dependency resolution | package/service 共用 declaration 与 artifact-root resolver，错误 taxonomy 一致 |
| PackageUnit serde/schema | 新 callable/service/boundary facts 必填，未知字段仍拒绝 |
| PackageUnit identity golden | build/local ABI/boundary ABI identity 输入分别稳定且由单一 owner 计算 |
| compiler artifact output/conformance | package artifact 含完整 typed code contract，不再只有 opaque empty effect |
| package test assembly | 新 PackageUnit 字段被校验并保留；service requirement artifact root 能传入 compiler |
| boundary/recoverable projection | 即时 linkable 与跨 request recoverable 分层，callback capability lifetime 明确 |

## 4. Delete and replace

允许删除且必须在提交说明中点名：

1. 断言 package manifest 出现 `services` 必然失败的测试。替换为 T04 的 package service
   requirement 正/反例。
2. 断言 `SourceEffectMetadata` 或 artifact effect 永远是 `Empty` 的测试。替换为 T08 的 effect
   推导和 T10 的 artifact emission 测试。
3. 只验证旧 opaque effect JSON shape、且没有仍成立语义的 golden。替换为 typed effect schema
   round-trip/unknown-field rejection。

本阶段不得删除：

- service source compile 测试；其 production path 要到 Phase 02 才切换。
- runtime router relay 测试；其 production path 要到 Phase 04 才删除。
- recoverable value 的 DB/spawn/cross-request 测试；本阶段是澄清分层，不是删除 recoverable。

## 5. Add

### 5.1 Input / resolution

- package 顶层 `services` 解析成功；alias 可用于 typed service call resolution。
- alias 重复、id/version 缺失、version range、artifact root 缺失、protocol mismatch 均失败。
- 同一 declaration fixture 同时驱动 package 与旧 service source 输入，证明单一 owner。
- requirement 不含 provider package id；build id 只允许作为编译输入 artifact 完整性验证，不成为
  PackageUnit 的 service call address。

### 5.2 Effect / link

- pure callable；caller-reachable mutation；返回参数 alias；alias escape；同 heap identity 比较；
  callback/stream；已知 native；未知/external callee；递归与互递归 fixed point。
- 未知 effect 保守向 caller 传播；diagnostic 包含调用链，但 analysis DTO 不保存文本调用链作为
  identity 输入。
- source 顺序、map iteration 和并行编译不改变结果。

### 5.3 Boundary projection

- ordinary data 得到 detached graph plan。
- mutable/alias-sensitive helper 为 Local Code ABI 可用、Boundary ABI 不可用。
- request-scope `any I` 和 native handle 得到 callback capability plan；缺 operation adapter 时
  fail closed。
- 相同 callback capability 进入 recoverable lane 时失败，除非 canonical contract 明确存在稳定
  resolver；Phase 01 默认没有此例外。
- stream、error、cancel/timeout contract 进入 boundary identity。

### 5.4 Artifact / identity

- PackageUnit 所有新字段 round-trip、unknown-field rejection、缺必填字段 rejection。
- diagnostic detail 变化不改变 identity；reason code/operation contract 变化会改变 boundary
  identity；implementation body 变化只按既定规则影响 build identity。
- artifact identity CLI 与 compiler projection 得到同一结果。

### 5.5 Integration

- 一个 package fixture 同时含 package dependency、service requirement、local-only helper、
  boundary-capable callable，编译后逐项核对 typed artifact。
- `skiff test` 对带 service requirement 的 package：缺 `--service-artifact-root` 时失败，提供有效
  root 时完成编译/artifact 装配；不要求执行 service call。

## 6. 按任务的最小验证范围

| 任务 | 聚焦范围 |
| --- | --- |
| T00 | `git diff --check`、service contract/version/revision术语 `rg` |
| T01 | `git diff --check`、文档链接/术语 `rg` |
| T02 | `cargo test -p skiff-artifact-identity`、single-source gate |
| T03 | artifact-model + artifact-identity |
| T04 | compiler-input + package/service manifest filters |
| T05 | compiler-compiled + projection-input + compiler boundary/DAG gate |
| T06 | compiler-projection 的 package artifact tests |
| T07 | compiler-projection + publication-abi 的 boundary/recoverable tests |
| T08 | compiler-compiled 的 effect/link analysis tests |
| T09 | compiler-projection + publication-abi 的 boundary projection tests |
| T10 | compiler source/lowering/projection/emission/driver 的 artifact-output filters |
| T11 | runtime-package-test + test-runner package tests |
| T12 | `phase-plan.md` §7 的完整阶段 gate |

任务文件中的命令优先于本表的概括。若实际 test target 名称不同，Agent 可以用 `cargo test -p
<crate> <filter>` 收窄，但必须在交付记录中列出实际命令和结果。
