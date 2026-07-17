# P1-T02A：Canonical Identity Dependency DAG Contract

## 背景与目标

T02 已把 publication/type semantic identity 的 derivation 与 validation 收敛到
`skiff-artifact-identity`，但 compiler crate-DAG contract 没有同步：

- `skiff-compiler-input` 在读取外部 service dependency artifact 后、进入 trust boundary 前调用
  canonical publication validator；
- `skiff-compiler-compiled` 在 source/compiled 到 projection facts 的唯一 handoff 中生成 canonical
  interface type key；
- facade crate 的测试却仍直接 dev-depend foundation identity crate，绕过 compiler adapter boundary。

前两条是有意的 foundation dependency，不是把 identity 算法下沉到 compiler；第三条是应删除的测试旁路。
本任务让代码依赖与 DAG checker 对同一边界达成一致，使后续 T06/T08 的 crate-DAG gate 可真实验收。

## 依赖与 worktree

- 依赖 P1-T02。
- 必须从已包含 T02 的 phase checkpoint 建 task worktree。
- 建议 branch：`codex/package-service-p1-t02a-identity-dag-contract`。
- 建议 worktree：`/Users/geek/workspace/skiff-p1-t02a-identity-dag-contract`。

## 架构决定

1. `skiff-artifact-identity` 是 foundation owner。允许 `skiff-compiler-input` 直接依赖它，只用于在外部
   artifact 被接受前调用 canonical validate API；input 不得拥有 preimage、hash、prefix 或 framing。
2. 允许 `skiff-compiler-compiled` 直接依赖它，只用于 compiled-to-projection handoff 的 canonical semantic
   key API；compiled 不得复制 `TypeRef` canonicalization 或 identity framing。
3. `skiff-compiler` facade 不直接依赖 foundation identity crate。facade integration tests 通过已有 compiler
   subcrate identity adapter调用 canonical owner；adapter只能是一跳 re-export/直接委托，不能含算法。
4. `compiler-input-model`、`compiler-projection-input` 等纯 DTO 层仍不得依赖 identity crate。不得为了 gate
   通过而把 foundation edge 扩散到所有 compiler crates。

## 完成态

1. `check-compiler-crate-dag.mjs` 的 final graph 显式允许且只允许上述 input/compiled 两条新增 normal edge，
   注释说明 trust boundary 与 projection handoff 的理由。
2. checker self-test 同时证明两条 edge 合法，并证明相邻 DTO crate 的同类 edge仍被拒绝。
3. facade `Cargo.toml` 删除 `skiff-artifact-identity` dev dependency；facade tests 不再直接 import该 crate。
4. input/compiled production callsite 仍直接调用 canonical API，没有本地 fallback、hash、prefix拼接或
   serde-based preimage。
5. 不改变任何 identity projection、prefix、wire schema、artifact内容或运行时行为。

## 写入范围

- `scripts/check-compiler-crate-dag.mjs` 及其内建 self-tests。
- `compiler/Cargo.toml` 与直接使用 identity 的 facade integration tests。
- 为维持一跳 adapter 所必需的 compiler subcrate public re-export。

不要修改 `artifact-identity` 算法、compiler input/compiled 业务逻辑、runtime/router 或 artifact fixtures。

## 验证

```bash
node scripts/check-compiler-crate-dag.mjs --self-test
node scripts/check-compiler-crate-dag.mjs
cargo test -p skiff-compiler-input -p skiff-compiler-compiled
cargo test -p skiff-compiler --test artifact_output
node scripts/check-artifact-identity-single-source.mjs
git diff --check
```

若 `artifact_output` 过重，可先跑直接受 import 迁移影响的 test selector，但提交前必须至少完成
`cargo check -p skiff-compiler --tests`。

## 反向搜索与自验收

```bash
rg 'skiff_artifact_identity' compiler/tests compiler/Cargo.toml
rg 'canonical_json|sha256|IDENTITY_PREFIX|:sha256:' compiler/input compiler/compiled
```

第一条在 facade tests/Cargo 中应归零；第二条只允许与本任务无关且已有明确 owner 的命中，每个新增或剩余
identity production 命中必须解释。回报 DAG edge 矩阵、checker self-test、聚焦测试、commit 与 clean 状态。

## 停止条件

- 需要复制 identity 算法才能移除 dependency。
- 需要让 DTO crate普遍依赖 identity foundation 才能让 checker通过。
- callsite 语义显示 input 并非 trust boundary，或 compiled 并非 canonical projection handoff。

出现停止条件时先回报，不得用临时 normal-edge exception；checker 明确禁止 normal dependency exception。
