# P5-F41：R05 Generation Lifecycle Harness

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点F41，依赖I30 PASS及D41 COMPLETE。当前为implementation checkpoint；D41确认缺失R05真实A/B
transcript入口，但production Router/Runtime/compiler/store/isolated owner已足够，无需设计或公共ABI变化。
F41完成并合流后只解除全新I31 cheap combined；I31 PASS后才解除全新R05真实probe。

## 写入范围

新增：

- `scripts/run-package-service-generation-lifecycle-smoke.mjs`
- `scripts/lib/package-service-generation-lifecycle-smoke-real.mjs`
- `scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs`
- `scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs`
- `scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs`
- `scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs`
- `test-runner/fixtures/package-service-websocket-generation-a/{package.yml,api.yml,main.skiff,main.test.skiff}`
- `test-runner/fixtures/package-service-websocket-generation-b/{package.yml,api.yml,main.skiff,main.test.skiff}`

允许窄改：

- `scripts/lib/package-service-ecosystem-smoke-real.mjs`：只导出已有open/message/close/deadline helpers，不改变
  single-generation语义。
- `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`：只把package coordinate/test name与
  `expectedGeneration`变为带旧默认值的显式参数，旧caller行为不变。

禁止修改Router、Runtime、compiler、deployment、artifact schema、公共ABI、activation/acquire/release语义、
四对象或contract identity；禁止patch/re-sign artifact、组件fake、protocol peer、manual emitter、业务retry或
legacy/dual path。若上述任一项成为必要前置，立即报告`TASK_NOT_EXECUTABLE`和最小设计决策，不得扩张范围。

## 固定真实Transcript

唯一未来入口为：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

入口必须在同一isolated stack/artifact root中，以正常source和真实compiler/store依次author/store A、B：

1. activate A并等待generation 1 ready，连接client A；
2. activate B并等待generation 2 ready，确认active B且health pin count为1；
3. A连续send/receive两次，均观察`P5-R05-GENERATION-A-MARKER`；
4. 连接B并send/receive一次，观察`P5-R05-GENERATION-B-MARKER`；
5. 对B执行unary POST，断言HTTP 200及JSON B marker；
6. close B并等待pin count回到1；
7. close A并等待pin count、in-flight均为0且无pending activation；
8. 始终由isolated owner清理supervisor/instance/ports/lease/workspace。

一个覆盖完整run的deadline必须在A/B authoring前开始。A、B两次fixture Cargo失败都必须保留F26A有界、
脱敏、hash/bytes、最多三条diagnostic envelope并带candidate label。fixtures必须保持相同service
protocol/operation identity，但产生不同PackageBuildId、deployment revision、assembly identity和marker。

## 完成标准与验证Owner

开发Agent只运行direct evidence：

```bash
node --check \
  scripts/run-package-service-generation-lifecycle-smoke.mjs \
  scripts/lib/package-service-generation-lifecycle-smoke-real.mjs \
  scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs \
  scripts/lib/package-service-ecosystem-smoke-real.mjs \
  scripts/lib/package-service-ecosystem-smoke-oracle.mjs

node --test \
  scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs \
  scripts/tests/package-service-ecosystem-smoke-real.test.mjs \
  scripts/tests/package-service-ecosystem-smoke-lifecycle.test.mjs \
  scripts/tests/package-service-ecosystem-smoke-diagnostic.test.mjs
```

开发Agent不得运行新增真实入口、旧smoke、Router/runtime/instance、stable、完整gate或I31 combined。提交必须包含
自验收矩阵、反向搜索和未决风险。

F41合流后的全新I31唯一owner运行：

```bash
node --test scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs
```

该combined必须在同一临时artifact root中用现有fixture binary真实编译/存储A、B，跨Rust receipt到JS
oracle断言A/B build/revision/assembly identity不同、service protocol/operation identity相同且两份immutable
record仍可读；不得启动Router/runtime或执行完整transcript。

## 风险、证据与集成

风险为高：真实generation lifecycle test infrastructure及跨Rust/JS authoring boundary。F41 direct evidence因
上述scripts、fixtures、fixture binary或`ecosystem_smoke_fixture.rs`变化失效；I31因compiler/test-runner
authoring、canonical store、receipt schema、fixtures、JS oracle或Cargo.lock变化失效；最终R05还会因
Router/Runtime lifecycle、F23E wire、provisioning、isolated owner、activation/health或checkout/environment
source变化失效。

从integration当前HEAD新建独立worktree与分支，不得回滚唯一允许的integration untracked ledger。只提交F41
范围；不push、不merge main、不操作stable。首次实际代码修改须在启动后5分钟内发生，否则返回
`TASK_NOT_EXECUTABLE`。完成状态仍是implementation checkpoint，不得称为稳定候选或R05 PASS。
