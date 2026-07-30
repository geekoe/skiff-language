# P5-I16：V6 Replacement Platform Source Shared-target Probe Result

`I16 V6 PASS`

证据锚定`3ceb1cfa6a2f66b8b918a6df03718aaa40375e66` / tree
`b506f10a9d2e7f05e33e1c34b211e1b79b3e2626` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。`p5-i16-command-group-v2`的19项前置命令全部PASS；其中merge-only
Rust test为`1 passed / 0 failed / 0 ignored`。唯一dynamic combined从非repo cwd执行一次并PASS，没有重试。

- schema为`skiff-platform-source-shared-target-probe-v6`，`status`与`primary.status`均为PASS，`firstError`为null；
- 三轮origin为A→B→final-A；两个跨root artifact evidence各为34 before/after、四个targeted crate全Fresh、
  `changed=0/allowed=0/disallowed=0`；
- 4次identity probe的8个prelude/std值全部匹配golden；最终artifact 32个，structure为`stringsNoMatch=6`、
  `depInfoNoMatch=26`，registry exact为`[{id:"std",root:"std"}]`；
- combined没有调用dependency helper/install/tsx，不含own `nodeDependencies`字段；`fullProbeRuns: 0`、
  `hostAttempt: null`、`sourceSuite: null`；
- A/B worktree及Git admin/registry、owned task root/shared target、40个process group与temporary ledger全部ABSENT，
  foreign state preserved，cleanup/ownership errors为空。

G16D消费后，ledger已移到repo外归档
`/Users/geek/workspace/skiff-phase-05-evidence/p5-i16-3ceb1cf-v6-combined-ledger.json`；文件SHA-256为
`52bd4b04db92e95fbcb646d6c26656b4bcd4c25034dd500e7fc767ce9b01b05d`，内部digest为
`5196d144123a1a217d2bafb067bdeabb2cbe4bd27da2009f424ca73d0b3bda41`，production ledger路径已不存在。本结果只建立
该候选的replacement combined证据，不是R16、G16、F04或阶段verdict；后续Gate/evidence代码或候选变化会使其失效。
