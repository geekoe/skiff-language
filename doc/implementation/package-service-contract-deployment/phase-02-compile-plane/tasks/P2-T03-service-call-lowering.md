# P2-T03：Contract Requirement 与 ServiceCallRef Lowering

## 目标

让consumer compile只读取ServiceContract，实际调用才生成ServiceRequirement、稳定slot和ServiceCallRef；
彻底移除canonical compile path对provider ServiceUnit/build/PublicationAbi的依赖。

## 依赖与 worktree

- 依赖 T01 checkpoint。
- 建议 branch：`codex/package-service-p2-t03-service-call-lowering`。
- 可与 T02、T04 并行。

## 完成态

1. 新contract dependency reader在trust boundary严格解析、assign/validate ServiceContract；不解析
   serviceAssembly、ServiceUnit、PackageUnit或provider path。
2. ContractRequirement含alias、service coordinate、contract version、expected protocol identity；不含
   provider字段。
3. contract dependency reader建立严格的alias/type/operation typed index，供 T01 source carrier 和 lowering
   消费；未知/重复/identity mismatch fail closed，package-local nominal不能冒充contract type。
4. lowering为实际call site分配稳定serviceRequirementSlot，生成ServiceCallRef和usedOperations；未使用的
   declaration不产生ServiceRequirement。
5. File IR canonical external ref不再生产旧ServiceDependencySymbol/完整OperationAbiRef carrier；package
   direct call target保持原local implementation link语义。
6. 普通A→B→A service requirement cycle不需要provider compile closure；跨contract schema引用cycle拒绝。
7. 旧provider closure resolver退出canonical compile path，只允许T06 legacy adapter/runtime allowlist使用。

## 写入范围

- `compiler/input` contract dependency新模块及tests。
- `compiler/lowering` dependency operation index、service-call lowering/external refs及tests。
- 不修改 `compiler/source/**`；聚焦测试用 T01 冻结的 carrier 构造输入，最终 facade 接线由 T05 完成。
- 不修改effect analyzer、package projection或driver。
- 不修改artifact-model/identity公共wire；缺字段回报checkpoint owner。

## 验证

```bash
cargo test -p skiff-compiler-input
cargo test -p skiff-compiler-lowering
git diff --check
```

必须包含provider缺失仍可compile consumer、一个/多个operation slot稳定、unused requirement、protocol/type
negative和direct package call不受影响的测试。

## 回报

提交commit、自验收矩阵、旧provider读路径反向搜索、slot稳定性规则和File IR示例。不得保留“先加载
provider再丢弃”的伪独立路径。
