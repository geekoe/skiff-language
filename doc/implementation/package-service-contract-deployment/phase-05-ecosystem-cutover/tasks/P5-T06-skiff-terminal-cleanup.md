# P5-T06：Skiff Terminal Legacy Deletion / Checker / Canonical Docs

## 权威输入与DAG

- 设计：`doc/architecture/package-service-contract-deployment.md` §1–§15。
- 依赖：R02 PASS的exact Skiff checkpoint；与T07/T08/T09A–C可并行，解锁R03。
- 风险：中高；terminal public surface、checker完整性及stale canonical docs。
- branch：`codex/p5-t06-skiff-terminal-cleanup`。worktree：`/Users/geek/workspace/skiff-p5-t06-cleanup`。
- 当前共享状态是R02 PASS的Skiff consumer checkpoint；完成后只是Wave 3 partial candidate。使用新的开发
  Agent；证据对legacy production subjects、checker registry/mutations、verify接线或被替换文档变化失效。
- 五分钟内开始删除或迁移第一个已无consumer的真实owner；若R02仍有production import，
  立即报精确blocker，不留deprecated shim。

## 写入范围

独占 `artifact-model` / `artifact-identity` legacy modules/exports，runtime linked/eval中仅为旧DTO存在的
model/converter，`cross-system-fixtures/**`，新ecosystem boundary checker/self-test/verify接线，以及
`doc/reference/publication*.md`、`doc/architecture/release-registry.md`、runtime/router README。另独占F04移交的
`scripts/lib/encrypted-storage-live-harness.mjs`、`scripts/check-db-encrypted-storage-live.mjs`、
`runtime/encrypted-storage-live/**`、`runtime/live-tests/**`、verify runtime-live semantic fixture/plan、
`doc/architecture/test-runner-runtime-isolation.md`与相关AGENTS canonical命令。
root Cargo/lock与`scripts/verify*.mjs`在本任务独占。不改T02–T05已验收的production语义。
F16C已在encrypted-storage、runtime-live verify plan和isolated bootstrap落下platform-source-root transport；T06拥有这些
文件的后续四对象/legacy迁移，但必须保留该exact transport与tests，不得恢复ambient path。

## 完成态

1. 删除`PackageUnit`、`ServiceUnit`、`PublicationAbiUnit`、service assembly/bundle/index/build record及
   legacy identity/resolver/runtime-program modules的production type/module/export。无alias/deprecated wrapper。
2. runtime internal model若仍需要新canonical execution type，用职责命名并直接从PackageArtifact/
   RuntimeAssembly projection，不保留旧DTO shape或converter。
3. fixture disposition完整：迁移到四对象golden或删除并记录语义退役；test-only legacy
   只可作checker mutation string。
4. 新checker从canonical subject registry枚举Skiff production owners，旧DTO/reader/writer/converter/
   service-version selector/fallback/common aggregate零命中；omission、rename、move、duplicate、string camouflage、
   test-only import都有self-test mutation。
5. reference/architecture/runtime/router文档只描述package source、code-free contract、source-free deployment、
   complete assembly、Host ingress及replica。不保留历史兼容章节。
6. checker在default `checks`/`verify`中恰好执行一次，不重复已有identity/runtime checker责任。
7. encrypted-storage live harness使用四对象authoring、canonical activation/Host ingress和新test-runner CLI；删除
   legacy service.yml、instance sync/reload、service/version selector及旧env/config flags。non-live计划测试证明
   exact artifact/base assembly/runtime target；动态加密轮换证据由最终唯一live owner执行，不在本任务操作stable。
8. runtime-live tests迁为package/contract/deployment/assembly fixture，verify传exact base assembly与
   config/DB/file/http capability bindings；legacy `service.yml`与“只有新flags、无package root”的半迁移状态归零。

## 唯一聚焦验证 owner

```bash
cargo check --workspace
node scripts/check-package-service-ecosystem-boundaries.mjs --self-test
node scripts/check-package-service-ecosystem-boundaries.mjs
node scripts/verify.mjs --only checks --list
node --test scripts/tests/encrypted-storage-live-harness.test.mjs
node --test scripts/tests/verify-runtime-live-canonical.test.mjs
node --test scripts/tests/verify-live-plan-platform-source.test.mjs scripts/tests/isolated-test-runtime.test.mjs
git diff --check
```

不跑完整checks/verify。后两项只重建T06直接修改的F16C caller ledger；候选不变时不重跑I16 Host gate。提交一个commit
并合入Skiff integration branch，回报删除模块/fixture disposition、checker subject/mutation矩阵、docs替换索引、反向
搜索及自验收矩阵。
