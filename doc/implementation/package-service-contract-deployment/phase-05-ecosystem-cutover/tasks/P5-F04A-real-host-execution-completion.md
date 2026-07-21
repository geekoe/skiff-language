# P5-F04A：Real Host Execution Completion

## 输入、owner与限制

- 输入：D10完成；exact integration `1dc1d7ae037e5de45b68d8a82a2e84860f7fee15` / tree
  `0c448e6f64b2e85ca35199b82ef664d5e3a1e633`，已包含F04 implementation、F08/R08与shared lock。
- 独立worktree/branch，一个clean commit，不merge/push。完成后只解锁T05/F04原六项窄接收。
- owner限于environment config/renderer caller与直接tests，以及test-runner checked-in fixture、聚焦preparer、现有
  fixture binary薄dispatch、source-suite显式执行及Rust重复fixture收敛。`deploy-runtime-stack.mjs`仅可显式传`prod`。
- 不改source registry内容、compiler/effects/projection、Router/Runtime Host/wire、artifact/deployment writer、
  canonical store/runtime execution/assembly实现、WebSocket smoke、manifest或root lock；不操作stable。

## Environment完成态

`local-instance-config.mjs`唯一校验并保存environment：default `dev`，ASCII token 1–200，拒绝`.`、`..`、空白、
斜杠、非字符串与超长值。所有`renderRouterConfig` caller显式传值：instance=config.environment、local CLI=`dev`、
deploy=`prod`；renderer缺值fail closed。真实instance test必须证明自定义`f04-host-test`穿透到`router.yml`。

## Runnable fixture完成态

新增`test-runner/fixtures/package-service-host/**`正常contract/helper/provider/consumer source；helper直接same-heap
mutate fresh Box，consumer经payments detached contract，provider按输入分支返回，test精确断言
`provider-observed-helper-mutated`。

`test-runner/src/package_service_host_fixture.rs`独占preparation；现有fixture binary只解析互斥
`--prepare-host-base <fixture-root> --work-root <dir> --receipt <file>`并调用该模块。preparer只用production authoring
API，按contract→helper/provider→provider deployment→consumer→consumer deployment→assembly顺序发布，并输出schema
`skiff-package-service-host-fixture-v1` receipt，至少携带environment、exact helper/contract/provider/consumer/
deployment refs与base assembly identity。

Node只接受仓库内固定consumer root，严格校验receipt schema/environment/non-empty canonical assembly identity；receipt
位于isolated temp root。std registry/gate先原样运行，再显式prepare并调用现有runner `--base-assembly`；最终日志必须
显示checked-in consumer test PASS。Rust聚焦测试复用该fixture/preparer并删除重复临时authoring。

## 验证

```bash
node --test \
  scripts/tests/skiff-instance-config.test.mjs \
  scripts/tests/runtime-stack-config.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs \
  scripts/tests/isolated-test-runtime.test.mjs
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
cargo test --locked -p skiff-runtime-package-test --test package_artifact
cargo test --locked -p skiff-runtime-eval package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation
node scripts/run-skiff-tests.mjs
node scripts/run-package-service-ecosystem-smoke.mjs --replicas 2 --self-test
cargo clippy --locked -p skiff-test-runner --all-targets -- -D warnings
git diff --check
```

关键证据是`run-skiff-tests`越过supervisor ready、assembly activation与HTTP ingress后，checked-in consumer test PASS；
Rust assemble、Host compile、std PASS、self-test或源码字符串均不能替代。回报environment链、typed receipt、authoring
顺序、最终Host结果、source/commit/tree、single commit/clean/lock、reverse-search与extra-review。

## Implementation checkpoint与D11 handoff

允许范围已在`031c6b8a883aa92cb395cde565aeb00e0907dac6` / tree
`d2375116b5865acf80c7c0f7f820804ab8046bc4`形成single clean checkpoint，并由root合流为`7f368102`；lock保持
`f3ce5457138c58aec4c84abda431afa96013e3fd`。Node 23/23、Rust integration 12/12、package artifact 5/5、
same-heap eval、smoke self-test、strict receipt CLI与task-crate Clippy均PASS。

真实probe仍为NO-GO：environment已正确进入Router配置，但production Router在Runtime首个binary
`runtime.capabilities`关闭socket；随后binary/text activation与health还有连续不匹配。D11确认这是F03B endpoint
职责的DAG排序遗漏并冻结F09/R10。checkpoint合流不把F04A或F04称为complete；R10 PASS后必须原样恢复本节真实
Host gate，不能重写fixture或用Router/Host聚焦test替代。

R10 PASS并合流`ff7a4df`后原样恢复probe，wire/capability已连通，但health始终为committed generation-0 snapshot、
`capabilityConnections` connected、`replicas: []`，120秒readiness超时。D12确认Runtime从空admission state启动且没有
durable committed reader，只能发送capabilities；Router正确拒绝把它当participant。F10/R11提前拆出F03C committed
bootstrap/reconnect职责；R11前F04A/F04继续保持未完成。

R11 PASS并合流`efb2bbbe`后第三次probe已越过readiness并进入`[skiff-tests] running std`，随后Cargo因
test-runner crate同时有`skiff-test-runner`与`skiff-package-service-smoke-fixture`两个binary，而source-suite未传
`--bin`退出101。该直接caller缺口仍属F04A写域，冻结F04B只显式选择canonical runner并原样恢复真实Host gate。
