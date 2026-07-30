# P5-T09D：Internals Contract-First Build / Assembly Workflow Checkpoint

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§5、§9–§13。
- 依赖：T09A–T09C已合入Internals integration的exact commit；解锁R09。
- 风险：高；跨service共享build/store/assembly owner，不得由T10–T12各自复制。
- branch：`codex/p5-t09d-internals-workflow`；worktree：`/Users/geek/workspace/internals-p5-t09d-workflow`。
- 当前共享状态是T09A–C clean merged contract checkpoint；完成后交R09，不是production assembly candidate。
  使用新的开发Agent；证据对contracts、Skiff CLI/store、shared scripts/package scripts/tests或依赖变化失效。
- 五分钟内修改旧isolated graph或local package store owner；不改任何service implementation语义。

## 写入范围

独占 `internals/scripts/**`、`aihub/service/scripts/**`、共享package artifact preparation/store cleanup、
AIHub/Agine `package.json`中的build/test/type-check接线，必要root `AGENTS.md` 工作流。不改
Codex/AIHub/Agine `.skiff`源码、contract body、deployment body或platform registry。

## 完成态

1. workflow固定顺序为`select explicit service roots -> compile all packages independently ->
   validate all deployments -> collect exact deployment receipts -> resolve one generated assembly`；不再串行
   “完整构建provider service后才编译consumer”。
2. local store只写T01四类immutable records/pointers，不保留source-root symlink publication store、
   `publicationStorageSegment/discoverLocalPublications`或`--service-artifact-root`。
3. linked worktree type-check/test使用temporary isolated store/assembly及explicit `SKIFF_ROOT`，不写stable root/
   reload；main build/dev/start provenance guard继续fail closed。
4. workflow为后续actual deployments提供唯一显式roots/receipts closure接口与canonical fixture；不创建
   `assembly.yml`或虚构deployment identity，最终五service验收集合的generated RuntimeAssembly归T09E。
5. tests明确拒绝missing contract/package/deployment、duplicate provider/Host selector、partial build及stable
   worktree provenance违规。

## 唯一聚焦验证 owner

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t09d-cargo.XXXXXX)"
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
node --test \
  aihub/service/scripts/local-package-store.test.mjs \
  scripts/worktree-provenance.test.mjs \
  scripts/isolated-service-graph.test.mjs \
  scripts/test-isolated-service.test.mjs
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
  node scripts/prepare-canonical-assembly.mjs --list --fixture-only
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
git diff --check
```

不运行AIHub/Agine build/dev/start或stable reload。提交一个commit并合入Internals integration branch，回报workflow DAG、store owner、
old script/flag disposition、provenance证据及自验收矩阵。
