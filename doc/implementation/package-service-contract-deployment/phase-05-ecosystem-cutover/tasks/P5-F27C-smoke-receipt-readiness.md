# P5-F27C：Smoke Receipt / Readiness Convergence

依赖D38 complete，可与F27A并行。独占ecosystem smoke JS helper及直接Node tests，不改compiler/test-runner/Rust/Router
production。一个clean commit。

严格解析fixture receipt：exact keys、production/overlay/overlayRecordPath、完整contract/deployment refs、三个entrypoint及
固定method/host/path/name/selector互相绑定；bootstrap receipt必须含exact std identity/build/record/pointer。activation response
要求environment、exact assembly、committed/active tuple与generation 1。之后poll control health，条件为exact environment/
generation/assembly、无pending、healthy connected replica、matching capability connection；deadline有界，禁止业务WS retry。
ready后只创建一次WebSocket。保留F26A bounded diagnostic与cleanup；bootstrap Cargo显式`--locked`。

Node tests覆盖delayed-health正例及generation0/wrong tuple/pending/no replica/no capability/timeout/receipt mutations，断言失败不
创建WS、成功恰好一次。只跑专用Node tests/syntax/diff-check；禁止真实fixture Cargo/smoke/full/I16/Host/stable。
