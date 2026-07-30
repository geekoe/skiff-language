# P5-F27C：Smoke Receipt / Readiness Convergence

依赖D38 complete，可与F27A并行。独占ecosystem smoke JS helper及直接Node tests，不改compiler/test-runner/Rust/Router
production。一个clean commit。

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §2“不变量”第8、9条：replica必须加载完整同一assembly，且deployment revision、assembly identity与其它
  identity必须分开；
- §5“ServiceDeployment”：deployment receipt中的implementation、operation mapping、ingress selector及
  dependency binding必须保持typed且互相精确绑定，不能从display name或package path猜测；
- §12“RuntimeAssembly 与扩容”：每个environment只有一个active assembly，每个replica加载完整相同assembly，
  且assembly admission、health、drain和atomic reload必须在runtime层可观测；
- §13“Registry、Release 与 Publish”：不可变typed records与可更新pointer分离，publish必须先验证typed
  artifact再更新允许更新的pointer；
- §14“Fail-closed 条件”：service/package dependency、version或identity不匹配必须在进入请求路径前失败，
  runtime不得靠raw JSON或display name补事实。

本任务只把上述既有语义落实为smoke harness的严格receipt/readiness oracle，不新增control API字段，也不改变
activation或health的production语义。

严格解析fixture receipt：exact keys、production/overlay/overlayRecordPath、完整contract/deployment refs、三个entrypoint及
固定method/host/path/name/selector互相绑定；bootstrap receipt必须含exact std identity/build/record/pointer。activation response
要求environment、exact assembly、committed/active tuple与generation 1。之后poll control health，条件为exact environment/
generation/assembly、无pending、healthy connected replica、matching capability connection；deadline有界，禁止业务WS retry。
ready后只创建一次WebSocket。保留F26A bounded diagnostic与cleanup；bootstrap Cargo显式`--locked`。

Node tests覆盖delayed-health正例及generation0/wrong tuple/pending/no replica/no capability/timeout/receipt mutations，断言失败不
创建WS、成功恰好一次。只跑专用Node tests/syntax/diff-check；禁止真实fixture Cargo/smoke/full/I16/Host/stable。
