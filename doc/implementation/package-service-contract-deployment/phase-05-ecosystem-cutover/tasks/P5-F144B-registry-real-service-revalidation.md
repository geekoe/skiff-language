# P5-F144B：Registry 真实 Service 重验续作

状态：Ready

## 父节点

- 直接父节点：`P5-F149-authoring-reachable-package-closure-result.md`
- 原 consumer result：`P5-F144-registry-real-service-revalidation-result.md`

## 进入状态与写入

- shared std binding blocker已闭合。
- worktree `/Users/geek/workspace/skiff-packages-p5-f144b`，branch `codex/p5-f144b-registry-revalidation`。
- 只写 `registry/` 与 Registry 专属测试。

## 完成标准

- 20/20 intended operations Available，无额外 operation。
- 闭合四个 `*Put` 和四个 `*PointerCas` 的既定 callable provenance/value lifetime/effect，使其真实可投影；不得降低
  boundary规则或增加 compiler特例。
- 真实 immutable record、release pointer、active deployment pointer storage覆盖成功、冲突、CAS mismatch和missing。
- canonical `npm run test:registry` 在隔离 store、显式 integration `SKIFF_ROOT` 下通过。
- 无 legacy contract/deployment或 Registry privilege。

若需要共享 Skiff语义修改则停止。提交、不 push、不操作 stable。

