# P5-G16：F04 Real Host Gate Result

`G16 FAIL`

唯一一次`--mode full`调用锚定`10746a2b52e927a65fa30acc11533b2ef8f65a34` / tree
`933a074a126ac286f18e4e4da0215f8736ef810b` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`，exit 1且未重试。I16 combined ledger前后SHA-256为
`5e6e05c5dd15af50319b26f556b0e684361d3204bd412da2688185b1274db899`，digest为
`7f9d16db5d15f9b20e206b0eb96a0e7bcbd08a15c363ee01bd5462ec30198919`。

primary为`full A-origin/B-root changed shared-target artifact hash or mtime`。A-origin与B-root Cargo build均成功，
错误发生在Host入口前，因此`sourceSuite:null`、`fullProbeRuns:0`，std/Host/finalValue均未建立证据；真实完整Host累计仍为0。
full-mode调用计数已为1，不得在未审计/修复时直接重试。

失败ledger digest为`792a8a1da5678288093fbe5135978dee8ce3a33304a7b8169da6edc4323fc653`；但异常发生在
`ledger.fresh/artifacts`赋值前，且verbose Cargo outcome只在局部变量，所以没有保留exact artifact diff。这是独立证据
blocker，不得把聚合错误推断成production回归。

cleanup全部PASS：A/B路径与Git registry/storage、task root、21个PID、process groups、ports/listeners及lease均ABSENT，
foreign preserved、errors为空；前后tracked clean，唯一untracked仍为I16 ledger。未操作stable、编辑或提交。
