# P5-F157：Formal-index-aware Call Transfer

状态：Ready

## 父节点

- `P5-D90-codex-relay-post-date-semantic-audit-result.md`

## 写入与完成标准

- owner：compiler/source callable-effects summary/transfer及tests。
- callee return provenance/identity必须保留exact formal parameter indices，并在call site只映射对应actual。
- 返回第3个Fresh actual不得因第1个caller-reachable actual被污染。
- 真正返回第1个request actual仍标returnsCallerAlias/requiresSameHeapIdentity。
- 多分支返回不同formals、nested local calls、unknown/dynamic target继续conservative。
- 不放宽boundary eligibility，不改consumer。

运行callable-effects source聚焦tests与真实withRequestCors形状projection probe；格式、`git diff --check`。

worktree `/Users/geek/workspace/skiff-p5-f157`，branch `codex/p5-f157-formal-index-call-transfer`。
提交、不push、不操作stable。

