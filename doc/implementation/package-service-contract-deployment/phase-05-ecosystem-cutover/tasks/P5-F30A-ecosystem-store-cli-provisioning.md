# P5-F30A：Ecosystem Store CLI Provisioning

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、8、9条、§12、§13及§14。Router必须通过受信的显式
canonical-store adapter消费typed artifacts；启动配置不能依赖ambient cwd/PATH/env猜测，缺失或错误binary必须在接流量前
fail closed。

DAG节点F30A，依赖D40 complete及F03B/F03C合流；完成后解除I30 provisioning/consumer combined。风险高，验收分组为
Router canonical-store production provisioning。派发时给exact integration commit/tree/Cargo.lock。

唯一写入边界：

- Router config normalization/example及其直接tests、现有ecosystem-store client直接tests；不得修改F03B
  store/snapshot/gateway/pin production语义；
- `scripts/**`中dev-runtime paths、local/isolated instance config renderer、managed binary build/install、ordinary dev-init、
  runtime-stack remote deploy/PM2与直接tests；
- 不改compiler CLI/store实现、Runtime、F23E wire、四对象、Cargo manifests/lock或本机stable配置。

完成标准：

- Router删除`SKIFF_ECOSYSTEM_STORE_CLI`、`SKIFF_DEV_HOME`/cwd store-path fallback，只接受YAML与显式CLI override的绝对
  executable path；
- shared renderer要求并写入非空`ecosystemStoreCliPath`；
- canonical local path为`<devHome>/bin/skiff-compiler[.exe]`；instance与build-dev-runtime从当前checkout构建并通过
  `installManagedBinary`原子安装0755，isolated worktree不得越出自己的dev-home；
- remote Router部署闭包必须消费build-manifest compiler unit、上传/chmod exact binary并写远端绝对path；
- 删除PM2 unsupported `--release-mode`，release mode只来自YAML；ordinary dev-init删除legacy rewrite并写同一compiler path；
- direct example显式写path；missing/non-executable/wrong binary及manifest缺compiler fail closed；
- 不引入PATH fallback、Node store writer、隐式checkout path或test-only默认。

快速验证命令：

```bash
pnpm --dir router exec vitest run tests/config.test.ts tests/ecosystem-store-client.test.ts
pnpm --filter @skiff/router type-check
node --test scripts/tests/runtime-stack-config.test.mjs scripts/tests/runtime-stack-deploy.test.mjs scripts/tests/skiff-instance-config.test.mjs scripts/tests/isolated-test-runtime.test.mjs scripts/tests/managed-binary-lifecycle.test.mjs
git diff --check
```

所有测试组必须非零。只运行direct Node/Vitest/syntax与必要fake-Cargo lifecycle；禁止真实build/deploy、启动instance/
Router/runtime、R05 transcript、I02/full/I16/Host/stable。不改doc，不merge/push，一个clean commit。config renderer、
managed install、deploy manifest、Router config/store client变化会使证据失效；完成后仍是Implementation Checkpoint。
