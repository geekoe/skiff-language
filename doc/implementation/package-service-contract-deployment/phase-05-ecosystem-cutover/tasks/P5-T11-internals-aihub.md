# P5-T11：AIHub Package / Deployment / Client Cutover

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§12、§14–§15。
- 依赖：R09 PASS exact checkpoint；与T10/T12并行，解锁R03。
- 风险：高；contract-owned LLM schema、stream service call、HTTP/WS ingress。
- branch：`codex/p5-t11-aihub`；worktree：`/Users/geek/workspace/internals-p5-t11-aihub`。
- 当前共享状态是R09 PASS的contract/workflow checkpoint；完成后仍是Wave 3 partial candidate。使用新的
  开发Agent；证据对AIHub/Codex contracts、shared workflow、owned source/package/deployment/client/tests变化失效。
- 五分钟内修改package/source/API/deployment真实owner；不改T09B contract或T09D workflow文件。

## 写入范围与完成态

独占 `internals/aihub/**`，排除T09B contract、T09D已冻结的`service/scripts/**`/package scripts。

1. 旧service source root迁为Package root，public API包含contract映射需要的HTTP/WS、managed LLM、
   web search、provider catalog callables；删除`service.yml`。
2. provider boundary直接使用AIHub ContractTypeId types；与`llm-api` package internal models之间建立
   显式双向wrapper，测试证明结构相同但owner错误会fail closed。
3. AIHub只通过Codex ServiceContract编译`codexRelay/relayProxy.responsesCompletedResult`，不读
   Codex package/deployment；operation errors/stream/cancel保持contract语义。
4. deployment恰好映射所有contract operations，将Codex service requirement slot绑定到exact contract/
   deployment selector，ingress使用`aihub.localhost`，config/state/resource/policy完整。
5. deployment以SecretRefBinding闭合`bailian/deepseek/gemini/openai/openrouter apiKey`与
   `codexRelay.relayKey` requirements；secret值只在activation input解析，不进入ServiceDeployment、
   RuntimeAssembly、日志或fixture snapshot。missing/unknown secret ref在admission前fail closed。
6. client HTTP/WS URL只使用AIHub Host，无service/version query/header；内部service call测试仍断言
   router selector不存在。
7. source/package可在Codex implementation不存在时依赖已发布contract独立compile。
8. owned AGENTS/README/example config只描述Host ingress与SecretRef path，不保留service selector或把
   示例secret值写入artifact；不新建README。

## 唯一聚焦验证 owner

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t11-cargo.XXXXXX)"
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
  npm --prefix aihub/service run type-check
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
  npm --prefix aihub/service test
node --test aihub/service/client-url.test.mjs
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
git diff --check
```

不运行build/dev/start/stable reload。提交一个commit并合入Internals integration branch，回报operation mapping、contract/package wrapper、
dependency与secret-ref binding（只列path，不列值）、Host route反向搜索、测试及自验收矩阵。
