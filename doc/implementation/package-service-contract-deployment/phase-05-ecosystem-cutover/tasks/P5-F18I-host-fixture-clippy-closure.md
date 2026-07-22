# P5-F18I：Host Fixture Clippy Closure

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§10、§14；F16C、R15 result与D20。从D20
docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18i-host-fixture-clippy`、
`codex/p5-f18i-host-fixture-clippy`。使用全新开发Agent；这是候选机械质量blocker，一个clean commit，
不merge/push/stable/Host；五分钟内修改。

exclusive write set：`test-runner/src/package_service_host_fixture.rs`及其现有/child direct tests。禁止`#[allow]`、改公开CLI/
receipt/fixture语义、canonical_package、scripts、Router/Runtime、manifest/lock。

完成态：把F16C新增的8参数private helper收敛为聚焦typed input/context（或等价单一私有结构），caller只构造一次；
`platform_sources`仍与artifact/source/work/environment等exact绑定。不得通过allow、tuple位置参数或复制第二builder绕过lint；
行为、receipt/golden与public signature不变。

```bash
cargo clippy --locked -p skiff-test-runner --all-targets --no-deps -- -D warnings
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
cargo fmt --all -- --check
git diff --check
```

global fmt三个未触碰runtime/host baseline可精确单列；changed file必须fmt。回报commit/tree/lock、clippy exit、12+1结果、
fixture/receipt no-diff与extra-review。完成后由全新R15B只复验此exact blocker，不重跑readiness语义矩阵。
