# P5-T02：Authoring / Registry Client / CLI / Dev Sync Cutover

## 权威输入与DAG

- 设计：`doc/architecture/package-service-contract-deployment.md` §1–§5、§9–§11、§13–§15。
- 依赖：R01 PASS的exact T01 checkpoint；与T03–T05同级，解锁R02。
- 风险：高；production authoring/store/publish/watch consumer。
- branch：`codex/p5-t02-authoring-tooling`。worktree：`/Users/geek/workspace/skiff-p5-t02-authoring-tooling`。
- 当前共享状态是R01 PASS的implementation checkpoint；完成后仍只是R02 batch candidate。使用新的开发
  Agent；证据对T01接口、owned production/tests、CLI fixture或依赖变化失效。
- 五分钟内产生第一个code edit；无法只消费T01接口时立即回报 `TASK_NOT_EXECUTABLE`。

## 写入范围与非目标

独占 `compiler/**` 的production binary/authoring consumer，`scripts/skiff.mjs`、`skiff-dev-sync.mjs`、
`skiff-instance.mjs`、`scripts/lib/**`中build/publish/store/watch/dev registry及直接tests。新owner进入
新模块，不在已有85k/62k大文件复制规则。不改router/runtime/test-runner/artifact model
公共语义、verify接线或外部repo。

## 完成态

1. `package build/publish`、`contract build/publish`、`deployment build/publish`、`assembly build/activate`
   仅调用T01 typed boundary；发布结果是分离artifact/pointer receipts，不是common aggregate。
2. compiler有真实PackageArtifact/ServiceContract CLI入口；没有`cargo run`无bin失败，也不
   恢复旧service/publication compiler。
3. package contract coordinates解析已发布ServiceContract，provider不存在时consumer仍可编译。
   deployment projection只用contract/package artifacts，assembly只用root deployment closure。
4. dev registry/watch观测package/contract/deployment roots，产生完整assembly；先写所有immutable
   records，再CAS active pointer，最后调control reload。编译/验证/写入/reload失败不移动pointer。
5. 删除package publish的`skiff-cli-live-test-shim/packageUnitPath/packageUnitHash/abiIdentity`，旧
   `service dev`/service source authoring/pointer writer无production可达路径。
6. CLI help/error/docs只描述四对象流程；old `--service-artifact-root`/service assembly/index参数
   不作compatibility alias。

## 探针与唯一聚焦验证 owner

- contract-first build正例；missing/tampered contract、duplicate alias/provider、deployment mismatch负例。
- 用临时root执行两次sync，第二次stale generation失败且原pointer bytes不变。
- watch batch中任一root失败时不进行reload；成功时reload只收到exact assembly generation。

```bash
cargo test -p skiff-compiler
pnpm --dir scripts type-check
node --test \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-store.test.mjs \
  scripts/tests/package-service-dev-sync.test.mjs
node scripts/check-package-store-discovery.mjs
git diff --check
```

不跑完整`checks`/`verify`。提交一个commit并合入Skiff integration branch，回报CLI表、old command disposition、反向搜索及
自验收矩阵。
