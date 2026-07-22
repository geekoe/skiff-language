# P5-F21A：Gate Causal Evidence Result

`F21A PASS`

开发提交`916e2d5ff2a7860624d8000134ce23eea1a13f33`，parent
`7bb6c2af9517f2091654fd1f127e87ca6ef02f68`，tree `248861c9112f239a98106d1ee17e3fe26c2a6487`；bit-identical
合流提交为`9863575ed6abfa1bafdae256d276303f2994317e`，tree相同，lock保持
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

Gate ledger schema升为v6。Host failure不再保存单一`firstDiagnostic`：它按panic/error/invalid-result/failure等因果等级、
stderr优先于stdout的确定性heuristic及流内行号排序，generic
`[skiff-instance] stopping after startup failure`被降为secondary；最多保存3条诊断，每条UTF-8 excerpt上限512 bytes、
总上限1,536 bytes，并记录omitted count。排序不伪造两个pipe的跨流timestamp；路径、URL、secret与HTTP body继续脱敏，
原始行只保留SHA-256。validator从原始outcome精确重算诊断集合，改序、篡改或旧v5 ledger均fail closed。

F21A与F21B合流后的唯一batch combined锚定`dbfb98ac0a10d3959d803a8a92de1c04bba66fce`，运行三个相关Node test
文件共44 pass/0 fail；`git diff --check` PASS。未运行I16动态probe、Host/full/stable，本结果不是阶段verdict。
