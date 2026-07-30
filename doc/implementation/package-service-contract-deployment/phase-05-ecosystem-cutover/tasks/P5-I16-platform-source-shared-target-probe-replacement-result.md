# P5-I16：Replacement Combined Probe Result

`I16 PASS（后被F19A/B失效）`

证据锚定`10746a2b52e927a65fa30acc11533b2ef8f65a34` / tree
`933a074a126ac286f18e4e4da0215f8736ef810b` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。command group 12/12 PASS，merge-only为
`1 passed / 0 failed / 0 ignored`；dynamic combined从`/tmp`精确运行一次，exit 0且无重试。

- 三轮origin：A→B→final-A；四个targeted crates按合同clean。
- 4次identity probe、8个值全部匹配prelude/std golden；A-origin/B-root与B-origin/A-root均Fresh。
- 两组Fresh各记录34个hash/mtime artifact；final artifact 32个。
- structure为`stringsNoMatch=6`、`depInfoNoMatch=26`；registry exact为`[{id:"std",root:"std"}]`。
- A/B路径、Git registry/storage、task root、40个PID、groups、ports与temporary ledger全部ABSENT；cleanup/ownership
  errors为空，foreign preserved。
- `fullProbeRuns:0`、`sourceSuite:null`；完整Host当时累计0。

ledger SHA-256为`5e6e05c5dd15af50319b26f556b0e684361d3204bd412da2688185b1274db899`，内部digest为
`7f9d16db5d15f9b20e206b0eb96a0e7bcbd08a15c363ee01bd5462ec30198919`。同候选R15B、R18A/B/C、H18与R16均
PASS；随后G16在Host前暴露F19A/B范围，因此gate/isolated surface变化后本v3 ledger及窄验收不得再为新candidate解锁。
