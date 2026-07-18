# P2-T05C10G：Identity Checker Package-call Owner Coverage

状态：独立 checkpoint acceptance blocker；依赖 T05C10C/D/E/F，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package direct call”“依赖与
Identity”“Fail-closed 条件”。

## 目标与 ownership

- single-source checker 把 canonical package-call validator、identity consumer 和 emission consumer 纳入
  与 service-call validator 同级的 fail-closed owner graph。
- 独占 `scripts/check-artifact-identity-single-source.mjs` 及 self-test；禁止修改 Rust production/tests。

## 完成态

1. checker 验证 package-call validator 唯一存在于 artifact-model，并验证 target/table exact shape 规则由其拥有。
2. artifact-identity 与 compiler emission 只能 delegation 到 shared validator；第二定义、缺失 owner、缺失任一
   consumer delegation 都 fail closed。
3. owner existence、exclusive definition、consumer delegation 和 self-test fixture 从一份 canonical registry
   派生，不再维护多份平行手写清单。
4. self-test 覆盖 package owner missing、definition missing/duplicate、identity/emission delegation missing。

## 验证

- `node --check scripts/check-artifact-identity-single-source.mjs`
- checker `--self-test` 与真实 checker
- targeted fixture mutation、`git diff --check`；不运行 Rust/compiler/full gate。

提交 clean；回报 registry shape、负例矩阵与 checker 结果。
