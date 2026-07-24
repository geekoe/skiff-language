# P5-F153：HTTP Request Native Semantics

状态：Ready

## 父节点

- `P5-D87-codex-relay-first-unknown-call-audit-result.md`

## 写入与完成标准

- owner：`artifact-model/src/native_signature.rs`及其registry tests。
- 为精确binding keys `std.http.request.headers`、`std.http.request.cookie`登记既有调用语义：
  读取caller request输入；无mutation、escape、suspend或identity requirement；返回Fresh。
- 必须与native signature参数/返回对齐。
- custom/unknown binding、动态interface、其他未审计`std.http.*`不受影响，禁止prefix inference。

先列出并运行native semantics registry聚焦tests；格式与`git diff --check`。不运行完整gate。

worktree `/Users/geek/workspace/skiff-p5-f153`，branch `codex/p5-f153-http-request-native-semantics`。
提交、不push、不操作stable。

