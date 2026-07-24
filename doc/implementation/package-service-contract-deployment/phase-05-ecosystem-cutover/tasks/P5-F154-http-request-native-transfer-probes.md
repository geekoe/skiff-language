# P5-F154：HTTP Request Native Transfer Probes

状态：Ready

## 父节点

- `P5-F153-http-request-native-semantics-result.md`

## 写入与完成标准

- owner：compiler/source resolved target与callable effects/provenance tests；必要时compiler真实source projection test。
- 真实source/import中的`std.http.headers`与`std.http.cookie`必须解析为exact native bindings并传递F153 semantics，
  不产生invokesUnknownTarget/unknown provenance。
- 包含literal response的最小HTTP handler投影Available；沿local helper传递仍保持exact summary。
- dynamic any-interface、custom/未登记native继续Unknown。
- 默认只补test；若暴露同owner直接实现缺陷可最小修复，不触碰其他HTTP native或eligibility规则。

先列出并运行compiler/source callable effects与真实projection聚焦tests；格式、`git diff --check`。

worktree `/Users/geek/workspace/skiff-p5-f154`，branch `codex/p5-f154-http-request-transfer-probes`。
提交、不push、不操作stable。

