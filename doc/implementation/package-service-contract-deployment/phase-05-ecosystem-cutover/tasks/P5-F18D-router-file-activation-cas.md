# P5-F18D：Router File Activation CAS

权威设计：`doc/architecture/package-service-contract-deployment.md` §6.1、§11、§12、§14；D11/D12与D20 result。
从D20 docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18d-router-file-cas`、
`codex/p5-f18d-router-file-cas`。全新Agent、一个commit，不merge/push/stable/Host；五分钟内修改。

exclusive write set：`router/src/router/assemblyActivationStateStore.ts`、可新增内部shared reducer/file persistence模块及
`router/tests/assembly-activation-state-store.test.ts`。不改protocol/schema/coordinator/server/Runtime/scripts/manifest/lock。

完成态：同environment的完整read→pure reducer→temp write/fsync/close→rename→parent fsync由跨实例cooperative exclusive
file lock保护。lock用`wx`、nonce/PID/hostname/dev+ino；仅同host且PID不存在的完整stale owner可在bounded grace后按
identity回收，foreign/PID复用fail closed；不宣称NFS/distributed lease。File/Memory共用prepare/abort/commit reducer；
exact committed replay即使空ACK也幂等，首次commit仍要求非空且全prepared/connected。primary/cleanup错误均保留。

测试必须含20轮两个File实例不同prepare每轮exact一胜一冲突、same prepare幂等、File/Memory parity、corrupt/symlink/
reopen及temp/write/sync/rename/parent-sync failpoints；状态只能old/new canonical且重试收敛、无owned residue。

```bash
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- tests/assembly-activation-state-store.test.ts tests/active-assembly-reload.test.ts
git diff --check
```

回报commit/tree/lock、20轮ledger、failpoints、extra-review。锁、persistence、reducer不得继续堆进单一大文件；若真实部署
要求multi-host/NFS才升级设计，当前本机file owner不升级。
