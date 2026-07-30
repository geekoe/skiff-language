# P5-F28C：Current Prelude Regression Pin

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第9条、§3“Package 与 PackageArtifact”及§9“Compiler 与
Projection 流水线”。Prelude identity由当前validated platform manifest/source fingerprint决定，不能保持source变化前的旧值。

DAG节点F28C，依赖D39 complete，可与F28A并行；完成后解除I28 compiler分片。风险低，验收分组为compiler source regression。
精确production base为`8982107308c021fe9a72ad9446e1820395a0bc83`，当前规范prelude identity为
`skiff-prelude-v1:sha256:5166ba3c306e94624094e0736da821a1b653da5aace1ef8cee2fb654f4106699`，变化归因仅为
c277e45新增`std.websocket.WebSocketIngressEvent`的platform source/API fingerprint。

写入边界仅`compiler/source/src/prelude_registry/tests.rs`与
`compiler/source/src/prelude_registry/tests/p5_f18a.rs`。更新active regression pin及准确归因注释；不得改production
prelude算法、platform source、F27A/B、历史gate文档或ledger。直接运行这两个测试所属的最窄compiler-source test filter、
format/diff-check；禁止fixture/smoke/full/I16/Host/stable。一个clean commit。platform source或prelude fingerprint算法变化会
使证据失效。
