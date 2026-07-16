# P1-T07：拆分 Boundary / Recoverable Projection

状态：`ready`
类型：营地前置任务，行为等价
依赖：P1-T01
执行者：Boundary Projection Agent，一份提交

## 背景

`compiler/projection/src/recoverable_boundary.rs` 接近四千行，同时负责 type closure、identity
facts、DB/spawn lane、custom restore、native adapter、validation 和大量测试；
`compiler/publication-abi/src/lib.rs` 也混合 surface builder、operation/index/signature helper。
T09 要在它们旁边实现即时 Linkable Value projection，必须先拆清两层职责。

## 目标

按 T01 的 canonical 分层重组模块，但保持当前 recoverable artifact 和 Publication ABI 行为完全
不变。拆分后即时 boundary projector 与 recoverable overlay 必须有不同 owner，共享的只能是
typed type-closure/index 基础。

## 建议模块边界

```text
compiler/projection/src/recoverable/
  mod.rs
  inputs.rs
  type_index.rs
  value_plan.rs
  identity_facts.rs
  storage_lanes.rs
  adapters.rs
  validation.rs

compiler/publication-abi/src/
  lib.rs
  error.rs
  surface.rs
  builder.rs
  operations.rs
  indexes.rs
  signatures.rs
```

实际文件名可以调整。`recoverable` 模块不得被改名为泛化 `boundary` 后继续承载所有职责；T09
会建立独立的即时 boundary 模块。

## 范围

- 上述两个超长生产文件及其内部测试的模块拆分。
- 抽出可共享的 typed type index/closure traversal，但不改变输出 DTO。
- 更新 import/re-export。

## 非目标

- 不实现 `LinkableValuePlan` 或新的 boundary availability。
- 不更改 recoverable policy、capability flag、identity、schema 或错误 taxonomy。
- 不更新 package artifact schema；T03/T09 负责。
- 不把未来 remote transport 放入 compiler ABI crate。

## 实现约束

- shared traversal 必须无 recoverable lane policy；lane-specific rule 由 recoverable module 持有。
- service/package concrete code owner 的旧命名若仅是 API 名且改名会改变行为，可留到 T09；但
  不得新增新的 service-own-code 表述。
- `lib.rs`/`mod.rs` 只做 facade/re-export。
- 不生成重复 type index、operation id 或 signature canonicalization helper。

## 验收标准

- 两个原聚合文件成为小型 facade 或消失。
- recoverable metadata、identity facts、DB/spawn/custom/native plan tests 全部行为等价。
- Publication ABI builder/index/signature tests 全部行为等价。
- T09 可以只依赖 shared type index + artifact contract 创建即时 boundary projector。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-projection recoverable
cargo test --no-fail-fast -p skiff-compiler-publication-abi
node scripts/check-compiler-boundaries.mjs
git diff --check
```

## 停止条件

- recoverable rule 无法与普通 type-closure traversal 分离；
- 拆分必须改变当前 recoverable output/golden；
- T01 文档仍把 service 当 concrete code owner；
- publication ABI 与 recoverable 各自实现不同的同名 canonical type/signature 规则。

遇到最后一种情况，升级为独立“canonical type/signature owner”前置任务。

## 提交

提交信息建议：`refactor(compiler): split boundary projection domains`
