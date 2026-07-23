# P5-D41：R05 Real Transcript Entry Preflight Result

结论：COMPLETE。当前不存在可执行的R05真实A/B transcript入口；这是
scripts/test-infrastructure implementation缺口，不是已确认设计缺口。D41未作R05、R02或Phase
verdict，未运行构建、测试、smoke或任何instance/stable操作。

证据锚定handoff HEAD `09004e0b0ec613a4e843f936f9b2190eb1be83b0`、production commit
`4a7b145396dc1359d0581d06e0bda1c31718504f`、production tree
`e0202d962d2580a89871bf5066972d3787b70714`及Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

## Blocking Facts

- 现有`run-package-service-ecosystem-smoke.mjs`只author一份fixture、执行`0→1` activation并验证一个
  generation marker；`--self-test`是纯内存fake，二者都不能作为R05入口。
- 当前没有两份contract-compatible且marker可区分的正常A/B source fixture。
- 当前没有同一isolated owner内两次author/store、`0→1→2` activation、A/B连接与unary、最后pin
  release/drain的orchestrator。
- 现有activation oracle只接受`0→1`，现有deadline也未覆盖fixture authoring起点。
- production owner已足够承载该transcript：isolated runtime可在同一run复用stack/artifact root，
  fixture binary可写canonical immutable records，Router health公开active/pending/replica及聚合
  `connectionPinCount`/`inFlightCount`，F30A compiler sidecar来自当前checkout并安装到isolated
  dev-home。所缺仅为test-infrastructure orchestration。

one-replica且连接顺序固定时，以`active B + pin=1`、B connect后`pin=2`、B close后`pin=1`、A
close后`pin=0`和`inFlight=0`闭合最后pin释放与drain；不需要新增公共diagnostic。若要求
generation-indexed retired-context公开diagnostic，或改变activation/acquire/release、四对象或identity
语义，必须升级为设计决策。

后续实现边界、固定transcript、direct tests、I31 cheap combined及证据失效面由
`P5-F41-r05-generation-lifecycle-harness.md`冻结。
