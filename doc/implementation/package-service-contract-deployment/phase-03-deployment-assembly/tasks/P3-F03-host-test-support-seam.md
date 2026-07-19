# P3-F03：Host Test-support Terminal Seam

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§6、§9、§12、§14。
- 执行输入：T07/T08A合同，以及 T07在同步 T08A后的编译 blocker：
  `runtime/host/src/program.rs` 仍 re-export 已删除的 `PackageOperationSymbolRef`。
- 风险/验收组：低风险机械 compile seam；由 T07三个 filter与 T09 runtime gate覆盖，不新增独立验收。
- 当前成熟度：T08A已合流，T07 implementation checkpoint已提交但聚焦测试被 host test-support seam阻塞。
- 有效证据状态：只影响 host tests/test-support compile；T08A五 crate check与 T07静态证据不失效。host program
  facade或 linked-program public API变化会使本修复证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent合流并同步 T07。

## DAG 与执行约束

- 依赖：blocker已在 T07同步分支复现。
- 解锁：T07 `assembly_admission` / `atomic_reload` / `request_entry` filters。
- branch：`codex/p3-f03-host-test-support-seam`。
- worktree：`/Users/geek/workspace/skiff-p3-f03-host-seam`。
- 五分钟内产生真实代码 edit；此前不跑测试、不扩大审计。若发现多个独立 host production seam，回报
  `TASK_NOT_EXECUTABLE` 与最小新 owner，不吞并 T07或 Phase 04。

## 写入范围与完成态

- 只修改 `runtime/host/src/program.rs` 及为删除该单一 stale re-export所需的同目录 test-support facade。
- 删除已不存在的 `PackageOperationSymbolRef` export/import及直接编译引用；不替换成 legacy alias、不新增 adapter。
- 不修改 `runtime/host/src/loader/**`、request entry、T07状态机、linked-program、其它 runtime crate或 Cargo/lock。
- host tests/test-support facade可编译，反向搜索该 symbol在本 owner归零。

## 唯一验证 ownership

```bash
cargo check -p skiff-runtime-host --tests
rg -n '\bPackageOperationSymbolRef\b' runtime/host/src/program.rs
git diff --check
```

若 host check继续被不同 owner阻断，给出 exact file/symbol，不越界修复。

## 回报

提交一个 commit，回报 commit、删除证据、命令和任何 remaining owner blocker。
