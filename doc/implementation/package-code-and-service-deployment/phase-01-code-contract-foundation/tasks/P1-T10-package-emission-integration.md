# P1-T10：接通 Package Compiler Production Path

状态：`ready`
类型：Compiler end-to-end integration
依赖：P1-T04、P1-T05、P1-T09
执行者：Compiler Integration Agent，一份提交

## 目标

把 package manifest 的 service requirements、typed service call refs、effect/link analysis 和
boundary projection接入 source → lowering → compiled → projection → emission → driver 的正式路径，
使真实 package build 产出 T03 定义的完整 PackageUnit。

## 生产数据流

```text
package.yml services
  → shared compiler-input resolver
  → SourceCompileModel typed service bindings
  → existing structured ServiceDependencySymbol/operation ref lowering
  → CompiledPublication code analysis facts
  → PackageProjectionInput
  → package boundary projector
  → emitted PackageUnit + identities
```

如果当前 service source path 已有可复用的 typed service-call lowering，应抽成共同 owner并复用；
不得复制一个 package 分支。编译器生成 contract/facts，不生成用户可见 stub package或临时代码。

## 必须删除/替换的旧行为

- 删除 package compile policy 对 `requires.services` / 顶层 `services` 的无条件拒绝。
- 删除或替换 `SourceEffectMetadata::Empty` 作为正式 effect 结果的路径。
- 旧 `ConfigAndEffectMetadata` 若仍用于 config，拆开 config 与 callable effect owner；不得 dual
  write 新旧 effect。
- package artifact emission 必须填充全部 required fields，不能依赖 constructor default伪装成功。

## 旧 Service Source Path

Phase 01 仍保留它，但有严格边界：

- 它消费共同 service requirement resolver、typed call refs、effect/link 和 boundary projector。
- 它把当前service source发布的具名public operation surface按T00/T03填入唯一
  `ServiceProtocolContract`；resolver不再从分散字段反推contract。
- 它可以继续生成当前 ServiceUnit/file artifacts，直到 Phase 02。
- 不允许新增 service-only effect、link、boundary 或 identity 实现。
- 如果旧 path 无法复用而必须永久复制规则，本任务停止；不以兼容为理由保留双 owner。

## 范围

按实际调用点最小修改：

- `compiler/source` compile model/policy
- `compiler/lowering` 的 service call binding glue
- `compiler/compiled` orchestration
- T05 拆出的 projection handoff
- `compiler/projection` package orchestration
- `compiler/emission`、compiler driver/pipeline
- compiler integration/artifact output/conformance fixtures

## 长文件约束

- 不向 `compiler/lowering/src/function_lowering.rs`、`compiler/driver/pipeline/mod.rs` 或其它超长
  文件加入新的 domain 算法；只允许小型调用点接线。
- 若需要实质修改其职责，先升级成独立前置拆分任务并更新 DAG。
- 同一 conversion/validation 出现第二份即停止，先抽 shared owner。

## 非目标

- 不改变 ServiceUnit ownership；Phase 02。
- 不让 runtime 执行 package 的 service dependency；Phase 03。
- 不生成 remote stub 或 router route。
- 不支持动态/optional/version-range service binding。

## 必须测试

- 真 package source + `package.yml services` 编译出 typed requirement。
- service call lower 为 structured dependency alias + operation ABI ref，不是字符串或 stub symbol。
- pure/mutable/alias/unknown callable 的 artifact boundary 状态正确。
- package build identity、local ABI identity、boundary ABI identity由 shared owner生成并通过验证。
- 旧 service source compile 仍通过且消费同一 shared facts。
- 无 service requirement 的普通 package 输出显式 empty requirements，而非缺字段。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler package
cargo test -p skiff-compiler artifact_output
cargo test -p skiff-compiler artifact_model_conformance
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

## 验收标准

- 真实 compiler entrypoint 而非仅 unit builder 产出完整 PackageUnit。
- `rg` 无 package 禁止 service requirement 的 production guard。
- effect/boundary facts 全程 typed，没有 raw JSON/diagnostic string bridge。
- 旧 service path 与 package path 无新规则重复。

## 停止条件

- production path 需要 provider deployment/build id 作为 service call address；
- source/lowering 无法表达 typed service operation ref；
- 接线必须复制 service-only analyzer/projector；
- 必须长期 dual-write old/new effect 或 artifact shape。

## 提交

提交信息建议：`feat(compiler): emit complete package code contracts`
