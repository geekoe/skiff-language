# P5-F387 Test-runner HTTP gateway convergence blocker

状态：TASK_SCOPE_EXPANDED（T1/T2 checkpoint完成；isolated activation被Router剩余v3门禁阻断）。

## 保留checkpoint

- worktree：
  `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch：
  `codex/p5-f386-package-test-http-gateway`
- T1 checkpoint：
  `4eccda2bea14fa2aa04bb3f2e0a2cdd3d788d85b`
- T1/T2 WIP：
  `804f2f9cf74e1ec14fc89b9a18ab20939b7e7e4d`
- result/HEAD：
  `71687e3765fc302611aad5de22a095d1621e4b8f`
- final tree：
  `73045a61aa165688b4d5a733199669f019242123`
- worktree clean；尚未合流。

已通过：

- runtime execution 27 tests；
- integration 23 tests，1 ignored；
- bins check；
- v2 Node receipt 4 tests；
- format/diff check。

isolated suite已成功启动临时Mongo、Router与Runtime，但assembly activation返回400：

```text
RuntimeAssembly.resolvedContracts[0].serviceProtocolIdentity is invalid
```

F387执行时Router三处仍为v3。F392随后已把snapshot/deployment snapshot迁到current v4，但
`router/src/protocol/runtimeProtocol.ts`仍是独立shared protocol门禁。后继必须先迁移该owner，再从
clean checkpoint引入F392/后继并重跑真实isolated suite。
