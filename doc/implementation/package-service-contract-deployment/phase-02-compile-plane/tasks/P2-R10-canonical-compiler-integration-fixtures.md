# P2-R10：Canonical Compiler Integration Fixtures

状态：rebuild；旧 R10 worktree 的混合未提交 diff 只作只读证据，按本任务重新选择 canonical fixture，
不得整体提交或移植。

依赖：T05、R03、R04、R06、R11、R13 已合入 terminal integration checkpoint。R13 未完成前不得迁移
logical DB fixture，避免测试先定义 production schema 语义。

## 背景

T07 首次 compiler test compile 发现，多个 integration suite 共用的旧
`build_service_publication` harness 把源码编译、service protocol、provider binding 和部署壳合并为
一个 fixture。这不是 T07 可以修补的 API 拼写；用空 contract 兼容会恢复已删除的旧
owner。因此本任务作为独立前置，先收敛测试架构，再恢复 T07 gate。

## 目标

1. compiler integration tests 不再依赖旧 service publication compile owner。
2. package 源码、type/effect、DB、File IR 和 compile diagnostic 测试使用 canonical package
   compile/test-support fixture。
3. 只有真正验证 service protocol 或 contract conformance 的测试才使用
   显式 `ServiceContractDefinition -> ServiceContract` fixture。
4. 旧 harness 中属于未实现部署壳的断言不伪装成 package 行为；保留在真正 owner 上的
   等价覆盖，或通过明确替代映射证明可删除。
5. canonical fixture 只返回 `PackageArtifact`、精确 File IR/resource 与 canonical package graph；
   不返回 `PackageUnit`/`ServiceUnit`，不携带空 `runtime_units` 或其它兼容槽位。

## 实现边界

- 先列出所有旧 harness call sites，按以下三类记录迁移去向：
  - package compile semantics；
  - explicit service contract/conformance semantics；
  - obsolete deployment-shell assertion。
- 迁移映射必须同时覆盖 `build_temp_service_publication*` 等间接 helper 的全部
  integration targets，不能只清理精确旧函数的八个直接调用。
- 拆分共享 helper 的责任，但不复制 canonical compile pipeline、identity、dependency resolution
  或 effect/projection 规则。
- 允许修改 compiler tests、test-support 与直接 fixture；不修改 production compiler/runtime 语义。
- 不修改 `compiler/driver/service_publication_tests.rs`；其旧 production owner/test disposition 由 T05 独占。
- 若某个测试只能靠恢复 provider inference、service aggregate 或 fake contract 通过，停止该用例并
  回报，不在本任务中发明兼容层。
- `http_routes`、service DB namespace/collection mapping、HTTP resource policy 等未来 deployment/runtime
  语义从 Phase 02 测试面移出，在它们的终态 owner 落地后重建；不保留旧 adapter 让测试继续通过。
- 不为缩短时间整批删除 Cargo test targets。任何删除必须在 commit 证据中给出旧断言到
  现有/新测试的映射。
- T05 已完成 `compiler/driver/service_publication_tests.rs` 的逐项 disposition；R10 不重新迁移或恢复该文件。
- `compiler/tests/http_routes.rs` 属于 deployment/ingress 语义，Phase 02 删除或移出 target，待 Phase 03/04
  以终态对象重建；不由独立兼容任务接管。

## 完成态

1. production 与 compiler tests 中精确旧函数 `build_service_publication` 及其间接 helper
   反向搜索为零，不保留 allowlist。
2. `cargo check --tests -p skiff-compiler` 通过，且所有保留的 canonical compiler integration targets
   仍可编译；删除/延后的 target 有逐项 disposition，不要求旧 route/deployment target 继续存在。
3. 至少运行 package compile fixture 与显式 contract fixture 的直接测试；有删除时运行替代覆盖。
4. 新 helper 命名和输出类型反映单一责任，不返回 package+contract+deployment 聚合对象，
   也不嵌入空 legacy runtime holder。
5. 没有 production 语义 diff；targeted rustfmt 与 `git diff --check` 通过，worktree clean。

## 聚焦验证

```bash
cargo check --tests -p skiff-compiler
cargo test -p skiff-compiler --test service_conformance
cargo test -p skiff-compiler --test <migrated-package-suite>
rg -n '\bbuild_service_publication\b' compiler
rg -n 'build_temp_service_publication' compiler/tests
git diff --check
```

完整 compiler gate 仍只由 T07 在 R10 合入后运行一次。
