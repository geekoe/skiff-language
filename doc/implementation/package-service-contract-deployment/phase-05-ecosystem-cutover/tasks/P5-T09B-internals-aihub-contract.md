# P5-T09B：AIHub Code-Free Contract / Contract-Owned LLM Schema

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §4–§8、§10、§14。
- 依赖：R02 exact Skiff checkpoint；与T09A/T09C并行，解锁T09D。
- 风险：高；AIHub是Agine compile contract，现有boundary错用package nominal types。
- branch：`codex/p5-t09b-aihub-contract`；worktree：`/Users/geek/workspace/internals-p5-t09b-aihub-contract`。
- 当前共享状态是R02 PASS的contract checkpoint输入；完成后只解锁T09D合流。使用新的开发Agent；
  证据对Skiff contract schema/CLI、该contract/schema fixture/tests变化失效。
- 五分钟内新增code-free contract authoring；不通过`api.yml`或provider callable自动漂移contract。

## 写入范围与完成态

只写 `internals/aihub/service/contract.yml`、contract-owned schema fixture/聚焦验证。不改service
implementation、package/deployment、client、package scripts或共享Internals workflow。

1. contract包含Agine真实消费的managed LLM stream、web search、provider catalog，以及AIHub对外
   HTTP/WS ingress需要的operations；映射以stable key/ContractOperationId为owner。
2. `LlmRequest`、`LlmStreamEvent`、`WebSearchInput`、`WebSearchResult`及闭包类型在contract schema
   自包，不使用`agine.ai/llm-api` PackageTypeId/结构相等替代ContractTypeId。
3. typed error/stream/cancel语义精确；callback/native/persistent不需要的lane不被误开放。
4. 隐藏AIHub、`llm-api`、`llm-providers`实现source/artifact后contract仍能独立build/publish。

## 唯一验证 owner

```bash
P5_ARTIFACT_ROOT="$(mktemp -d /tmp/skiff-p5-t09b.XXXXXX)"
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t09b-cargo.XXXXXX)"
P5_SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint
git -C "$P5_SKIFF_ROOT" status --short
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT="$P5_SKIFF_ROOT" \
  node "$P5_SKIFF_ROOT/scripts/skiff.mjs" contract build aihub/service \
  --artifact-root "$P5_ARTIFACT_ROOT" --json
node --test aihub/service/contract.test.mjs
git -C "$P5_SKIFF_ROOT" status --short
git diff --check
```

提交一个commit并合入Internals integration branch，回报operation/type closure表、package nominal反向搜索、provider隐藏正例及
自验收矩阵。
