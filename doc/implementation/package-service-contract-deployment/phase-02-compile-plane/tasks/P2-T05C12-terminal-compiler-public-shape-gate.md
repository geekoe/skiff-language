# P2-T05C12：Terminal Compiler Public-shape Gate

状态：A01 checker false-negative repair；依赖 T05C11，T07 evidence refresh 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Compiler 与 Projection
流水线”“不变量”。

## 目标与 ownership

- compiler structure checker 对 compiled/projection-input 的终态 public surface 与 ownership shape fail closed，
  能发现 publication aggregate/adapter 即使被改名。
- 独占 `scripts/check-compiler-boundaries.mjs` 及其 self-test/fixture；禁止修改 Rust production/tests。

## 完成态

1. gate 校验 compiled/projection-input 只暴露 frozen canonical public items/handoff，出现新的未声明 public
   aggregate/adapter即失败。
2. 以 renamed struct/function 负例证明：同时持有 compiled payload + publication metadata/config，或接受该
   aggregate再生成 projection input 的 shape 会失败；不能只增加名字 token blacklist。
3. canonical `CompiledPackage -> ProjectionInput` 通过；checker requirement/self-test从同一 registry派生。
4. 真实 checker与self-test PASS，无 transitional allowlist。

## 验证

- checker syntax/self-test/真实检查、fixture mutation、`git diff --check`；不跑 Rust/full T07 gate。

提交 clean；回报 public-shape registry、负例矩阵与 checker结果。
