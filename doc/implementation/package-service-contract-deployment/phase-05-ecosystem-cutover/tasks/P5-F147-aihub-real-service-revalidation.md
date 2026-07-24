# P5-F147：AIHub 真实 Service 重验

状态：Ready

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 写入与目标

- worktree `/Users/geek/workspace/internals-p5-f147`
- branch `codex/p5-f147-aihub-revalidation`
- 只写 `aihub/` 及 AIHub 专属 workflow fixture。
- Service API 只包含真正 public executable callable；interface declaration、instance method、internal helper 不得误投影。
- managed LLM `streamChat` 必须生成 Available ServerStream，item nominal schema完整。
- `config.dev.yml` 是 canonical service config authoring；无 contract/deployment legacy。

## 验证

- Linked worktree只运行 type-check/test/canonical workflow，使用 temporary store 与显式 `SKIFF_ROOT`；不得 build/dev/start。
- 验证 availability receipt、stream contract、正负 callable 分类、格式与 `git diff --check`。
- 若需要共享 Skiff 修改，停止并返回 blocker。提交、不 push、不操作 stable。

