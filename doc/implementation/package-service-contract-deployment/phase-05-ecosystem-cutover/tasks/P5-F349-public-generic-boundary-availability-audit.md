# P5-F349 Public generic boundary availability audit

状态：Ready（只读）。

## 直接父节点

- `P5-H35-external-ingress-surface-separation.md`
- generic冲突原始归因：`P5-F303-compiler-probe-failure-classification-result.md`

## 目标

只读确认`api.yml` public binding、PackageLocalAbi、PackageSchema与service-call availability怎样正确分层，
并把F302的四个std WebSocket generic declaration与external ingress分离。

必须回答：

1. F301当前在哪个production owner把任意public generic declaration升级为整包错误。
2. PackageLocalAbi/implementation link是否已经能发布、导入和使用generic declaration及fully applied
   nominal；哪些路径仍依赖PackageSchema。
3. 最小语义应是“public但schema unavailable”还是其它已有typed状态；不得增加exact std symbol特例。
4. Service-call operation引用generic declaration/applied nominal时如何结构化Unavailable；公开错误类型、
   dependency import和identity/golden如何保持fail closed。
5. `std.websocket`四个generic类型作为package-visible platform types时，哪些PackageSchema records应为零，
   external handler compilation从哪里取得类型。
6. F302 combined probe、两个eval source-inline失败及相关tests需要怎样闭合。

## 范围与写入

只读检查compiler source/core/projection/contract/input/lowering、artifact model/identity和std publication。
不得修改production/test/std/corpus/lockfile。

只允许新增：

- `P5-F349-public-generic-boundary-availability-audit-result.md`

result记录exact commit/tree、唯一first-loss owner、最小修复范围、负例、generation判断及F302重跑条件。
不运行workspace/stable/live，不push。提交result并返回commit。

## Worktree

- `/Users/geek/workspace/skiff-p5-f349-public-generic-audit`
- `codex/p5-f349-public-generic-audit`

