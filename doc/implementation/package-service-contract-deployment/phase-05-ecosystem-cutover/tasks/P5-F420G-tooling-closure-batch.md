# P5-F420G Tooling closure repair batch

状态：Ready（F420F 审计后的单一修复波次）。

## 直接父节点

- `P5-F420F-tooling-path-closure-audit-result.md`

F420F 在 exact candidate 上逐项执行 tooling phase 9–57，得到 37 phase PASS / 12 phase FAIL，
并把全部失败归并为五个互不重叠的 owner。没有待决设计问题；RuntimeAssembly v2 的 HTTP-only
方向已经由 F420B 与权威架构冻结。

## 精确起点与 DAG

- integrated repair start：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59`；
- tree：
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

五个叶子从同一 task-doc checkout 并行，写入互斥：

```text
F420G1 crate public API oracle
F420G2 dev-sync assembly identity fixture
F420G3 HTTP-only ecosystem/generation tooling closure
F420G4 test-runner target inventory oracle
F420G5 verify-plan single command owner
             ↓
合流后的 combined probe
             ↓
冻结 N4 candidate，由独立 gate owner 跑完整 N4
```

任何叶子不得运行完整 tooling、Router、test-runner Rust suite、`run-skiff-tests`、stable/live。
每个叶子分别提交 implementation/result，保持 clean，不 merge/rebase/push。

## 合流条件

- 五个叶子均按自己的 focused matrix 通过；
- 写入集合没有交叉或未授权 production 扩张；
- G3 不恢复 RuntimeAssembly WebSocket ingress、第三 entrypoint、旧
  `name/contract/operation` receipt 字段或兼容读取；
- G5 不放宽 duplicate execution 检查；
- 合流后由单一 integration owner 执行 F420F §9 的全部受影响 phase，随后才建立冻结候选。

