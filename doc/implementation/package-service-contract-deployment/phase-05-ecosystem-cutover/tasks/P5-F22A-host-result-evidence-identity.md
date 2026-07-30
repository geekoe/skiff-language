# P5-F22A：Host Result Evidence Identity

## 输入、设计引用与 DAG

唯一权威业务语义来自 `doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14，
以及 P5-F04/P5-F04A 冻结的真实正例：checked-in consumer test 必须经 production Host 执行唯一 assertion，并观察
`provider-observed-helper-mutated`。测试显示名和 ledger 字段不是公共 package/service 契约。

输入为 G16D 持久证据
`/Users/geek/workspace/skiff-phase-05-evidence/p5-g16d-3ceb1cf-v6-real-host-gate.json`、D30A/B/C 审计，
以及 checkpoint `3ceb1cfa6a2f66b8b918a6df03718aaa40375e66` / tree
`b506f10a9d2e7f05e33e1c34b211e1b79b3e2626` / `Cargo.lock` blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。审计已确定：production discovery 将
`main.test.skiff` 映射为 `main.__test`，runner 输出 `PASS {module_path}::{name}`；Gate 后加入的
`PASS main.test.skiff::provider observes helper mutation` 从引入起即错误，command-double 又复制了同一 oracle。
G16D Host child 实际 `code=0` 且结果为 `11/11`、`1/1`，但旧 ledger 把 12 条原始 PASS 全部替换成不可逆的
`PASS <unexpected>`，因此旧失败证据不得事后升级为 PASS。

执行 DAG：

```text
D30 aggregate + G16D FAIL
  -> F22A Host evidence repair
  -> one cheap combined
  -> fresh R22 acceptance
  -> replacement I16 v6 exact-candidate combined
  -> fresh G16E contract/owner, new-cycle full #2
```

## 写入 owner 与非目标

使用全新开发 Agent，从包含本合同及 D30/G16D checkpoint 文档的精确 integration commit 创建
`/Users/geek/workspace` 下独立 worktree/branch；交付一个 clean commit 后结束，不 merge/push/stable。

exclusive write set：

- 新增 `scripts/lib/platform-source-probe-host-evidence.mjs`，作为 fixture guard、Host result/PASS identity、
  final-value projection 的唯一 owner；
- `scripts/lib/platform-source-probe-evidence.mjs` 仅做上述 Host 逻辑的抽取与接线，保留 artifact evidence owner；
- 新增聚焦 `scripts/tests/platform-source-probe-host-evidence.test.mjs`；
- `scripts/tests/platform-source-shared-target-probe.test.mjs` 只调整集成 command-double；
- `scripts/tests/package-service-host-negative-probe.test.mjs` 只移除同源错误 PASS oracle。

不得修改 `test-runner/**`、fixture/`std/**`、source suite、shared-target orchestration、Gate diagnostic/contract/schema、
Router/Runtime/compiler、dependency preparation、manifest/lock 或 canonical design。不得改变 `11/11 + 1/1`、唯一 Host
test、唯一 assertion、最终值或一次 Host 无重试语义。若必须越过写集或改变公共语义，停止并报告设计决策。

## 完成态

1. Fixture guard 仍严格要求三行、唯一 `test "provider observes helper mutation"`、唯一可达 assertion
   `assert root.main.run() == "provider-observed-helper-mutated"`，且无 alternate pass path。
2. Gate 不再从 fixture 文件名复制 Rust discovery 的 module spelling。它从语法有效的
   `PASS <non-empty-module-path>::<test-name>` 解析 identity，以 exact test name
   `provider observes helper mutation` 选择目标；module path 是运行时观察值，不是 JS 重实现的业务 oracle。
   当前 production 形态 `PASS main.__test::provider observes helper mutation` 必须通过。
3. 在 exact `11/11` 后 `1/1`、Host command code 0/signal null、process/port evidence存在的同时，目标 PASS 必须恰好
   一条。missing、duplicate、wrong test name、空/非法 module、过长或 malformed PASS 均 fail closed；std 或其它
   unrelated PASS 不能冒充目标。
4. 唯一匹配成功后，ledger 的 `observedPassLine` 与 `finalValueEvidence.passLine` 保存实际、有界的完整 PASS 行，
   `sourceSuite.finalValue` 只从已静态验证的唯一 assertion派生为 `provider-observed-helper-mutated`；不能仅凭 count 或
   source 字符串硬编码成功。
5. 非匹配 PASS 不保存原始无界内容。`passLines` 对每条 unexpected line保存固定前缀与原行 SHA-256 token；匹配行必须
   先通过 ASCII identity grammar 与固定长度上限。stdout/stderr/output SHA及现有 v6 bounded diagnostics继续保留，
   不保存 raw transcript、secret 或 HTTP body。
6. 保持 v6 ledger 的既有字段集合和 validator 接线；本节点不升 schema。旧 G16D full evidence因 Gate source变化失效，
   但 P27R/R21C dependency/startup证据与 F21A/B parser/marker聚焦证据不失效。当前 archived I16 v6 combined因 candidate
   变化失效，后续必须重新生成。
7. production Host evidence 规则只存在于新 child owner。command-double 使用真实 `main.__test` happy shape以及独立
   wrong/missing/duplicate/malformed/oversized cases，不再把测试 helper 中的错误 literal当作 production oracle。

## 便宜验证与交付

禁止真实 Cargo build、dependency install、I16、H18、full、Host、source suite、runtime 或 stable。至少运行：

```bash
node --test \
  scripts/tests/platform-source-probe-host-evidence.test.mjs \
  scripts/tests/platform-source-shared-target-probe.test.mjs \
  scripts/tests/package-service-host-negative-probe.test.mjs
node --check scripts/lib/platform-source-probe-host-evidence.mjs
node --check scripts/lib/platform-source-probe-evidence.mjs
git diff --check
```

报告 matched/pass/fail、commit/tree/lock、真实/反例矩阵、bounded/hash evidence、反搜旧错误 literal与第二 parser、
extra-review、clean status。任何测试用例只调用自己的复制 parser、或 production owner仍混入 artifact comparator 文件，
均不算完成。

## 证据失效边界

F22A 会使 G16D full、其 Gate result projection、当前 archived I16 v6 combined 与旧 R21 对整体 Gate surface 的结论失效。
F22A 合流后先执行一次 cheap combined；R22 与 replacement I16 必须锚定同一最终 candidate。随后若 Host fixture/test name/
assertion、runner PASS formatter/discovery、Host evidence owner、shared-target orchestration、candidate/tree/lock 任一变化，R22、
I16 与后续 full 全部失效。G16E 是本新周期允许的第2次 full；失败不得再沿本合同重试。
