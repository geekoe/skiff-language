# P5-R09：Internals Contract / Workflow Checkpoint Acceptance

## 角色与精确输入

未参与T09A–T09D实现的只读验收Agent。阅读权威设计 §3–§10、§12–§15，
T09A–T09D任务合同，并检查主Agent提供的exact clean Internals commit/tree、Skiff R02
commit与已有聚焦证据。

不得修改、创建commit、实现service wrapper或运行stable/live gate。

## 必验完成态

- Codex、AIHub、Agine三份contract都code-free、独立publish、schema closure完整，不引用
  provider/package-local nominal identity。
- AIHub contract-owned LLM types及Agine public API owner分界精确；不用结构相等、display name、
  `api.yml`或provider source生成稳定contract。
- 实际production service-call operations都在contract；未使用operation有明确disposition；HTTP/WS入口
  都有可映射operation，不是只公开type/receiver。
- shared workflow先冻结contracts，再可并行compile implementations，最后deployment/assembly；无
  source-symlink store、old service artifact root、provider-first serial build或semantic default。
- linked worktree isolation/provenance继续成立；T10–T12的写入面非重叠，不需再改共享workflow。

## 输出

第一行 `PASS` 或 `FAIL`。分contract ABI/schema与shared workflow列blocking issues、non-blocking
follow-up、证据命令、动态缺口及残余风险。PASS才解锁T10–T12。
