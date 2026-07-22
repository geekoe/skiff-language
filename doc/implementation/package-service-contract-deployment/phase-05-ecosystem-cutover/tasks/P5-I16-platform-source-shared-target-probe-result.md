# P5-I16：首次 Combined Probe Result

`I16 PASS（后被F18J失效）`

首次证据锚定`ecc53ec27c493e692f03112ba7d951397fadd831` / tree
`a875735da9db53e5c426f816b1238622b4ba4bbc` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。command group 12/12 PASS，merge-only为
`1 passed / 0 failed / 0 ignored`；dynamic combined仅运行一次并PASS：A/B/final-A三轮origin、4次probe/8个golden、
cross-root Fresh、strings 6与dep-info 26个no-match、registry exact `[{id:"std",root:"std"}]`，全部临时资源清理。
`fullProbeRuns: 0`，完整Host计数仍为0。

ledger SHA-256为`38a59eb4ff6a5892e73509085e2ab46b093936d12e9d199cfb3ad44eff09ddc9`，内部digest为
`89253c96a634ffe8403f01713690256af8c02eca6b4b0697170c1903e6fe7462`。R15B在同候选上PASS；随后R18A发现
authoring在platform guard前创建artifact store，故F18J修改production/test surface后，本ledger与R15B结果不再能为
新候选解锁R16/G16。旧ledger只保留归档，F18J合流候选必须由全新owner重新执行一次combined。
