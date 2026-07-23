# P5-I48A：Fixture Projection Reacceptance

DAG节点I48A，依赖I48对冻结production commit
`ad847f7254521d1dd4679a4f8af72b2c88753310`、tree
`f0a33cc750025916df7b303e2f07b9db3f2e9c6d`的FAIL归类。I48除空选择器外的证据继续有效。

全新只读owner核验candidate后只运行一次：

```bash
cargo test --locked -p skiff-test-runner i02_spawn_submit_fixture_splits_unary_and_websocket_effects
git diff --check
```

测试必须实际执行且恰好覆盖fixture compile/projection：public spawn callable为suspending/cooperative，
WebSocket callable为non-suspending/not-cancellable，且保持同一contract/deployment。禁止编辑、提交、
重跑I48其他测试、真实I02/R05/instance/stable/full gate。PASS与I48既有有效证据合并后解除I02C。
