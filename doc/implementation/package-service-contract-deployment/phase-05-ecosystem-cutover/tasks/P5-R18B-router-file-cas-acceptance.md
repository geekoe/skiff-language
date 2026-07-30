# P5-R18B：Router File CAS Acceptance

使用未参与F18D、Router activation store、D20/I16或其它验收的全新独立只读Agent。权威设计：架构§6.1、§11、
§12、§14。输入为同一final candidate/lock、F18D ledger与I16 PASS bundle；前后clean且无Router writer。第一行只给
`R18B PASS/FAIL`。

必验：同environment完整read→pure reducer→temp/fsync/rename→parent fsync由跨实例lock包围；lock为
`wx`+nonce/PID/hostname/dev+ino，仅完整same-host/PID absent/bounded-grace stale owner可回收，foreign/PID reuse/
symlink fail closed。File/Memory唯一reducer，首次commit严格ACK，exact replay才允许空ACK；failpoint后仅canonical
old/new并可重试收敛，primary/cleanup均保留、owned residue为零；不宣称NFS lease、不改protocol/coordinator/server。

唯一抽查使用direct Vitest，禁止经会展开全量legacy Router suite的package script：

```bash
pnpm --dir router exec vitest run tests/assembly-activation-state-store.test.ts \
  -t 'serializes 20 rounds across two instances with exactly one winner and one conflict'
```

要求20/20 exact一胜一冲突；任何双成功/双失败/last-write-wins/residue即FAIL。不重跑其它F18D矩阵、Router/Runtime/
Host/stable。回报identity、证据复用、命令、extra-review与失效范围。
