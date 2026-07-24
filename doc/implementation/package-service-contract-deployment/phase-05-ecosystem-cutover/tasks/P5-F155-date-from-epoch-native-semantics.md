# P5-F155：Date.fromEpochMilliseconds Native Semantics

状态：Ready

## 父节点

- `P5-D88-codex-relay-next-unknown-call-audit-result.md`

## 写入与完成标准

- artifact-model exact binding `core.date.fromEpochMilliseconds`登记Fresh、所有may flags false，并与signature对齐。
- runtime/native registry canonical validation/probe确认handler仍精确注册。
- compiler/source真实wrapper resolved target/effects/provenance transfer为exact、no effects、Fresh。
- 未登记exact/custom native与dynamic receiver继续Unknown；禁止prefix inference。

允许artifact-model native registry、runtime/native registry test、compiler/source callable-effects tests；默认不改runtime handler。
运行三个owner聚焦tests、格式、`git diff --check`。

worktree `/Users/geek/workspace/skiff-p5-f155`，branch `codex/p5-f155-date-native-semantics`。
提交、不push、不操作stable。

