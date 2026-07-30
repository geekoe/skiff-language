# P5-F156：Exact Native Route / Context Parity

状态：Ready

## 父节点

- `P5-D89-native-route-context-parity-audit-result.md`

## 写入与完成标准

- owner：`runtime/native/src/registry/table.rs`及registry tests。
- parity validator仅允许exact `std.http.request.headers/cookie`为`None + Http`。
- 伪造HttpClient context或非Http route拒绝。
- outbound request/stream/sse与emitResponse既有context/route矩阵保持严格。
- full native semantics matrix、NativeRegistry构造/handler count与Date probe通过。
- 禁止prefix或route-wide exception。

运行runtime-native registry/matrix聚焦tests、格式、`git diff --check`。

worktree `/Users/geek/workspace/skiff-p5-f156`，branch `codex/p5-f156-native-route-context`。
提交、不push、不操作stable。

