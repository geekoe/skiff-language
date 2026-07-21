# P5-T08：Internals Registry / Platform Cutover

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §1–§5、
  §9–§15。
- 依赖：R02 PASS exact Skiff commit及T01 typed registry/pointer接口；与T06/T07/T09A–C并行。
- 风险：高；registry schema/persistence/CAS/audit，account/registry actual services。
- repo branch：`codex/package-service-phase-05`；integration worktree：
  `/Users/geek/workspace/internals-phase-05-integration`。task branch：`codex/p5-t08-registry-platform`，
  worktree：`/Users/geek/workspace/internals-p5-t08-platform`。
- 完整遵守Internals与`skiff-platform` subtree AGENTS；五分钟内编辑registry schema/service authoring。
- 当前共享状态是R02 PASS的external checkpoint；完成后只是Wave 3 partial candidate。使用新的开发Agent；
  证据对Skiff typed storage/pointer接口、registry wire/DB/service/client/tests或依赖变化失效。

## 写入范围

独占 `skiff-platform/package-registry/**`、`skiff-platform/account/**`、必要platform registry client/
generated service identity/Host route consumer。不改Codex Relay、AIHub、Agine或Internals共享isolated graph。

## 完成态

1. registry分别存储/验证PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly
   immutable records、各自typed release pointer history及T01 environment activation transaction audit；
   不有common artifact kind/domain aggregate。
2. package publish trusted build产出真实PackageArtifact；删除`packageUnitPath/packageUnitHash/abiIdentity`
   read/write/view/DB owner、`PublicationIdRead`及旧build complete payload。旧DB记录strict reject，不dual-read。
3. contract-first publish、package independent build、deployment validation、assembly activation四条API可分步执行；
   immutable write先于release pointer CAS；assembly activation只委托router coordinator的prepare/commit/abort，
   失败不产生partial committed pointer/history。
4. account与registry从旧`service.yml`迁为package/contract/deployment，HTTP callable显式进入Package API并
   完整映射contract operations；config/state/secret/runtime capability requirements有唯一binding。
5. account/registry使用不同Host，不在client/generated config/query/header传service/version selector；`POST /ping`
   在两Host下无selector collision。
6. package/detail/search/publish/resolve与contract/deployment/assembly API的真实正负例覆盖identity tamper、
   stale generation、authority/authz及audit history。
7. `skiff-platform/package-registry/registry-phase05-smoke.mjs`提供main-only live入口；其self-test使用fake
   transport断言四类publish/resolve/history最终结果，开发任务不得连接stable。
   同一脚本的`--prepare-test-assembly`模式从account/registry test-owned contract/package/deployment roots向
   temporary artifact root写完整canonical closure，并只输出assembly identity；它不是T09E production assembly。
8. owned `AGENTS.md`/README中的service.yml、service/version selector、旧publish/store指令同步改为canonical
   Host与四对象流程；不新建README，优先合入AGENTS。

## 唯一聚焦验证 owner

```bash
P5_ARTIFACT_ROOT="$(mktemp -d /tmp/skiff-p5-t08.XXXXXX)"
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t08-cargo.XXXXXX)"
P5_SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint
git -C "$P5_SKIFF_ROOT" status --short
P5_TEST_ASSEMBLY_ID="$(node skiff-platform/package-registry/registry-phase05-smoke.mjs \
  --prepare-test-assembly --artifact-root "$P5_ARTIFACT_ROOT")"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT="$P5_SKIFF_ROOT" \
  node "$P5_SKIFF_ROOT/scripts/skiff.mjs" test skiff-platform/account \
  --artifact-root "$P5_ARTIFACT_ROOT" --base-assembly "$P5_TEST_ASSEMBLY_ID" \
  --deny-skips --require-tests
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT="$P5_SKIFF_ROOT" \
  node "$P5_SKIFF_ROOT/scripts/skiff.mjs" test skiff-platform/package-registry \
  --artifact-root "$P5_ARTIFACT_ROOT" --base-assembly "$P5_TEST_ASSEMBLY_ID" \
  --deny-skips --require-tests
node --test skiff-platform/client/scripts/generate-services.test.mjs
node --test skiff-platform/package-registry/registry-phase05-smoke.test.mjs
git -C "$P5_SKIFF_ROOT" status --short
git diff --check
```

linked worktree不运行build/dev/start，不写stable package store/artifact root/reload。不跑live registry smoke。提交
一个commit并合入Internals integration branch，回报wire/DB/pointer history迁移表、旧字段反向搜索、service mapping、测试及自验
收矩阵。
