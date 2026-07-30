# P5-F370 HTTP gateway assembly generation correction

状态：Ready（F365唯一阻塞前置；F363共享seam窄修）。

## 直接父节点

- `P5-F363-runtime-http-gateway-execution-seam-result.md`
- `P5-F365-host-http-gateway-admission-wire-blocker.md`

父节点已确认target同时携带request-local activation handle与pinned deployment
`ActivationContext`；本leaf只纠正二者的generation语义，不改变canonical request wire或Host admission。

## Exact base与必须完成

- Skiff integration：`9c668bf9f8aa75f0ebfdf076915d5b5de1c4d327`
- tree：`fdf793aed683942f6ffcd783bbb35738e19e3c91`

1. `runtime/request/src/http_gateway_execution.rs::validate_request`必须把
   `routing.assemblyGeneration`与
   `target.eval().activation_context().identity().assembly_generation`比较。
2. 不得把`request_activation().generation()`重新定义为assembly generation；它仍是request-local
   generation，用于既有provider request ownership/lifecycle。
3. 增加直接回归：
   - 同一pinned assembly generation下两个连续、request generation不同的合法HTTP gateway request均通过；
   - wrong assembly generation仍fail closed；
   - wrong assembly identity、gateway identity及request metadata既有负例不退化。
4. 若正确修复要求改变`RuntimeAssemblyRequestStartFrameHeader`、ActivationIdentity、Host route或F363 target
   公共结构，立即返回`TASK_SCOPE_EXPANDED`。

## 写入、验证与交付

允许写入仅为：

- `runtime/request/src/http_gateway_execution.rs`；
- 对应`runtime/request`直接tests/fixture。

禁止Host、Router、transport DTO、artifact/deployment/compiler、test-runner、stable/live与lockfile。

```bash
cargo test -p skiff-runtime-request http_gateway -- --list
cargo test -p skiff-runtime-request http_gateway
cargo check -p skiff-runtime-request -p skiff-runtime-eval
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

selector必须非零。production/tests一个commit，result一个commit；clean，不merge/rebase/push。

- worktree：`/Users/geek/workspace/skiff-p5-f370-assembly-generation`
- branch：`codex/p5-f370-assembly-generation`
- 启动5分钟内开始修改；返回exact commit/tree、非零测试与自验收矩阵。
