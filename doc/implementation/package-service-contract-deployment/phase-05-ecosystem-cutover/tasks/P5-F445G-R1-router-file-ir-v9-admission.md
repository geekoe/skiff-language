# P5-F445G-R1 Router File IR v9 admission

状态：Ready。F445G required Router consumer child。

## 直接父节点

- `P5-F445G-timeout-artifact-lowering-link-checkpoint.md`

F445G implementation `dee2d0b5d67df9a6f3358d68ee835c7695680e21` 已把 File IR 持久格式从
v8/v6/v1 原子升级为 v9/v7/v2。Rust artifact/compiler/linker owners 已同步，但 Router
filesystem assembly loader 仍硬编码 `skiff-file-ir-v8:sha256:`，会拒绝 compiler 当前产物。

## 完成目标

只闭合两个已审计的 Router consumer：

1. `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
   - File IR identity admission prefix 改为 exact `skiff-file-ir-v9:sha256:`；
   - 不同时接受 v8，不增加 fallback 或兼容分支。
2. `router/tests/compilerGeneratedManifestCompatibility.test.ts`
   - compiler-generated artifact 的 File IR identity 断言改为 exact v9；
   - 保留 loader 从当前 compiler 记录完整装载成功的覆盖。

不得修改 Rust、compiler fixture generator、PackageArtifact/RuntimeAssembly schema、其它 identity
版本或 Router 行为。

## Test-first 与验证

在 F445G implementation base 上先证明 direct compatibility test 的 RED 来源是 Router v8
reader/expectation，再做上述两处最小修改。

至少运行：

```bash
pnpm --dir router test -- tests/compilerGeneratedManifestCompatibility.test.ts
pnpm --dir router type-check
pnpm --dir router test
rg -n 'skiff-file-ir-v8' \
  router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts \
  router/tests/compilerGeneratedManifestCompatibility.test.ts
git diff --check
```

direct test、type-check、Router full 必须通过；反搜必须零匹配。若当前 compiler-generated fixture
不是 v9、或出现第二个 production consumer，停止并如实上报。

## worktree 与提交

worktree：

`/Users/geek/workspace/skiff-p5-f445g-r1-router-file-ir-v9`

branch：

`codex/p5-f445g-r1-router-file-ir-v9`

base：`dee2d0b5d67df9a6f3358d68ee835c7695680e21`，再 cherry-pick 本任务文档。

提交 implementation，再只新增并提交：

`P5-F445G-R1-router-file-ir-v9-admission-result.md`

最终 clean。不得派子 Agent、merge/rebase/push、stable/live/network。
