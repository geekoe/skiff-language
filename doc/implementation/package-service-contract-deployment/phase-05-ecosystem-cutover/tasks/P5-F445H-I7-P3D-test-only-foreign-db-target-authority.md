# P5-F445H-I7-P3D Test-only foreign DB target authority

状态：`READY_FOR_DOCUMENTATION_CUTOVER`。

## 1. Parent chain and purpose

I7 M已经把Internals测试迁移为ordinary `kind: test` service。真实Relay编译在subject顶层DB operation
target lowering失败；Agine还存在同一direct topLevel/transitive package type的exact selection前置阻塞，
其foreign DB operations尚未到达。P3只读preflight确认现有consumer target、linker与runtime仍存在
metadata复制或按名字扫描的缺口。P3D只先冻结权威合同，不修改production或tests。

```text
I7 M test-service checkpoint
  -> P3 read-only preflight
  -> P3D authority cutover
  -> P3 compiler/linker/runtime implementation
  -> Relay/AIHub/Agine isolated reacceptance
```

## 2. Frozen baseline and ownership

| 项 | 值 |
| --- | --- |
| Skiff baseline commit | `b4275a48548bb21b8294d089c9108f7142609b40` |
| Skiff baseline tree | `1a34b604cf6ddf23628c7fdb0cc469aaa1a9ef4b` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-p3d-foreign-db-docs` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p3d-foreign-db-docs` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检核对了仓库AGENTS、I7 M result、P3报告冻结结论及当前artifact/link/runtime代码事实。
现有authority没有要求topLevel DB target复制provider metadata或使用名字作为identity；旧DB architecture中
“merge package DB metadata into service unit”的描述属于待删除的重复owner，不是需要保留的代际合同。

## 3. Frozen target and metadata contract

1. 仅`kind: test` service可在`access: topLevel` dependency上访问文件顶层`db object`；其语法、可见性
   和精确artifact约束与function/type顶层访问一致，不传递到transitive dependency。
2. Consumer File IR中的foreign `DbTargetIr`复用
   `PackageSymbolRef { package: Dependency(alias), symbolPath, abiExpectation }`。
   `PackageRequirement.expectedPackageBuild`提供精确provider build约束。
3. Assembly/linker通过`PackageBinding`选择exact `PackageArtifactRef`，再按
   `PackageArtifact.implementation_links.types[symbolPath]`定位provider `FileIrRef + typeIndex`，
   并核验provider File IR中同type的`declarations.db` attachment。
4. Linked/runtime唯一target identity是
   `DbObjectTargetId(PackageArtifactRef, FileIrRef, typeIndex)`。`typeName`只允许用于诊断，不参与lookup。
5. Collection、key、field、retention、lease、index与recoverable metadata只从provider File IR读取一次；
   consumer File IR、PackageArtifact和linked executable不得复制。两个dependency拥有相同module/type时
   仍按artifact identity区分；物理collection mapping继续由dependency edge独立投影和校验。
6. 该identity覆盖所有DB read/write target、`DbQuery`、lease claim、lease state read与claim write guard。
   Transaction本身没有target，内部operation分别携带target。
7. 缺失requirement/binding/type link/provider file/type/DB attachment、ABI/build mismatch或
   cross-artifact substitution全部fail closed，不得按短名、suffix或全图扫描回退。
8. 不改变File IR v9、PackageArtifact v9、Package local ABI v7或ServiceContract v5代际；Skiff未发布，
   直接替换旧target行为，不保留dual-read、fallback或compatibility。

## 4. Write scope

本任务只修改：

```text
doc/reference/testing.md
doc/reference/db.md
doc/reference/static-semantics.md
doc/architecture/db-capability-architecture.md
doc/architecture/package-service-contract-deployment.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/
  phase-overview.md
  phase-plan.md
  tasks/P5-F445H-I7-P3D-test-only-foreign-db-target-authority.md
  tasks/P5-F445H-I7-P3D-test-only-foreign-db-target-authority-result.md
```

禁止修改production、tests、fixtures、schema constants、Cargo或其它repo；不运行build/test/live/stable，
不访问network、Mongo、OAuth或browser。

## 5. Completion and downstream handoff

P3D完成要求：

- reference与architecture表达同一test-only可见性、symbol→binding→provider declaration链；
- provider File IR成为DB metadata唯一事实源，linked/runtime identity不含display name；
- 所有DB target surface与fail-closed矩阵完整；
- 代际不变与权限不扩张明确；
- Markdown fences配对、`git diff --check`与正反向搜索通过；
- task/result交Skiff integration steward合流。

P3D PASS只解除P3 production implementation，不代表I7 M动态blocker已关闭，也不恢复Relay、AIHub或Agine
isolated证据。
