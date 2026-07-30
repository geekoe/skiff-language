# P5-F158：Native HTTP Structural Contract Types

状态：Ready

## 父节点

- `P5-D91-codex-relay-post-formal-index-audit-result.md`

## 写入与完成标准

- compiler/projection对exact canonical HTTP Native names复用artifact-model canonical structural ContractTypeRef，不输出未知Builtin。
- 覆盖HttpRequest、HttpResponse、HttpResponseStreamEvent及server-stream item。
- imported PackageSymbol与Native两条真实路径生成同一canonical contract shape/identity。
- 同名非canonical、错误arity、HttpClient类型继续fail closed。
- contract normalization与service contract compile真实probe通过。

允许artifact-model helper必要公开化、compiler/projection与真实integration tests；不改normalizer接受未知builtin。
运行projection、contract normalization/compile聚焦tests、格式、`git diff --check`。

worktree `/Users/geek/workspace/skiff-p5-f158`，branch `codex/p5-f158-native-http-contract-types`。
提交、不push、不操作stable。

