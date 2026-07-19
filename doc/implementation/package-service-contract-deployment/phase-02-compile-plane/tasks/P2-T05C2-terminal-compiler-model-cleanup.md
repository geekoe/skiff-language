# P2-T05C2：Terminal Compiler Model Cleanup

状态：checkpoint repair split；R06/R13 合流后与 T05C1、T05C 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“核心结论”“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 背景

T05C1 首轮 production boundary 定位证明，旧 owner 不只存在于 facade/input 与 core，还残留在
input-model、source、lowering，且 source 仍通过 normal dependency 持有 publication-ABI crate。

## 目标与 ownership

- 物理删除 `compiler/input-model/**`、`compiler/source/**`、`compiler/lowering/**` 中已断链的
  publication/service common model、producer 与 adapter。
- 删除 source/lowering 到 publication-ABI 的 production edge；edge 归零后物理删除 orphan
  `compiler/publication-abi/**` 及必要 workspace/Cargo/Cargo.lock 声明。
- 独占上述 model crates 与 publication-ABI 最终 disposition；禁止修改 compiler facade/input、core、
  projection-input、projection/emission、driver pipeline、checker 与 integration tests。

## 完成态

1. canonical source/lowering 只处理 PackageSourceModel/LoweredPackage 及已冻结的 contract call facts；
   PublicationInput/Kind、Compiled/LoweredPublication、PackageUnit/ServiceUnit/service assembly producer归零。
2. source/lowering 不再 normal/dev/build 依赖 publication-ABI；orphan crate 不留在 production tree、workspace
   或 Cargo.lock 中，也不通过 feature/exception 保留。
3. 不改变 R06 requirement closure、R13 DB schema、T02 effect 或 T03 service-call lowering 的 canonical
   语义；只移除并收敛旧 owner与直接 tests。
4. 不恢复 legacy/compatibility adapter、provider inference、runtime witness 或 used-symbol closure。
5. integration tests/test-support 的断链由 R10 处理；本任务不修改旧 fixture 或 Cargo test target。

## 验证

- input-model/source/lowering 聚焦测试与必要 `cargo check`。
- production boundary checker；合流前只允许 T05C1/T05C 写域的精确命中。
- crate metadata/反向依赖搜索证明 publication-ABI edge 与 crate 均归零。
- targeted rustfmt、`git diff --check`；不运行 compiler integration tests或 T07 完整 gate。

提交并保持 worktree clean；回报删除模型/crate、保留的 canonical 测试、checker 剩余命中和聚焦证据。
