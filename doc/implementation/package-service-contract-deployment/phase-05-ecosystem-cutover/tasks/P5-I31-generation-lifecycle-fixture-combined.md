# P5-I31：Generation Lifecycle Fixture Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点I31，依赖F41在integration commit
`c808586546fddc5550f1caf7e520e849162a0946`合流。当前是implementation checkpoint；I31是合流状态上唯一
cheap combined owner，只验证A/B真实author/store及Rust receipt到JS oracle接线，不作R05/R02/Phase verdict。

全新只读Agent在上述exact commit/tree及Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`运行一次：

```bash
node --test scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs
```

必须确认测试在同一临时artifact root中使用现有fixture binary真实编译/存储A、B，并断言：

- A/B PackageBuildId、deployment revision及assembly identity不同；
- service protocol/operation identity完全相同；
- immutable A、B records均仍可读；
- 不启动Router/runtime，不执行完整transcript，不操作stable。

禁止编辑、提交、修复、运行F41 direct tests、真实新入口、旧smoke、instance或完整gate。若命令失败，只归类
blocking fact、失败层级与最小implementation owner；不得顺手修改。PASS只解除全新R05 Agent运行一次合同冻结的
真实命令：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

compiler/test-runner authoring、canonical store、receipt schema、A/B fixtures、JS oracle、Cargo.lock或checkout
source变化会使I31证据失效。
