# P5-F19A：Gate Artifact / Evidence Convergence

## 输入与 owner

权威设计：架构§3、§6.2、§9–§14；G16 FAIL与D25 result。使用全新开发Agent，从D25 docs checkpoint建立
`/Users/geek/workspace/skiff-p5-f19a-gate-evidence`、`codex/p5-f19a-gate-evidence`。一个clean commit；不merge/push/
stable，不运行combined/I16/H18/full/Host或真实Cargo build。

exclusive write set：`scripts/lib/platform-source-shared-target-probe.mjs`、
`scripts/lib/platform-source-probe-{support,contract}.mjs`、可新增单一artifact/evaluator child module，以及
`scripts/tests/platform-source-shared-target-probe.test.mjs`。不得改compiler/test-runner production、isolated runtime、
CLI、fixture、manifest/lock。

## 完成态

- combined仍对identity round的完整稳定artifact hash/mtime严格相等；full使用命名的mode-specific comparator：四个
  targeted crates必须只有Fresh、不能同时Dirty/Compiling；binary/rlib/hashed dep-info等受管payload与mtime不变。
  只允许记录并验证root-specific顶层dep-info的A→B materialization，差异必须可由exact worktree root替换解释；不能
  全局忽略`.d`或放宽production artifact。
- before、Cargo targeted lines/outcome、after与结构化diff在断言前写入ledger；stable payload变化、缺Fresh、冲突unit均
  fail closed且保留首个exact path、before/after hash/mtime/size/classification。抽取单一artifact evidence owner，删除
  第二比较器或散落条件。
- full Host command一经发起立即记`fullProbeRuns:1`并保存code/signal/output digest；nonzero、signal、count/parse failure
  仍保留attempt/outcome与primary-first，cleanup secondary独立。Host前失败仍为0。
- success必须从真实输出解析exact一个
  `PASS main.test.skiff::provider observes helper mutation`及11/11、1/1；ledger的finalValue/evidence来自该行，不能硬编码
  冒充观察结果。不可达expected assertion + `assert true`必须被拒绝。
- ledger schema升为v4并同步canonical validator；旧v3 combined显式失效。不得改变full最多两次的阶段预算。

## 便宜验证与交付

扩充command-double表覆盖：仅合法顶层`.d`root materialization PASS；binary/rlib/hash/mtime变化FAIL；missing或
Fresh+Dirty冲突FAIL；nonzero/signal/malformed/extra counts/wrong或missing PASS line；Host后primary+cleanup双失败；每个
失败ledger必须保留diff/outcome/attempt。另用旧I16 ledger做纯离线A/B/A分类corpus，不构建。

```bash
node --test scripts/tests/platform-source-shared-target-probe.test.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
node --check scripts/lib/platform-source-probe-support.mjs
node --check scripts/lib/platform-source-probe-contract.mjs
git diff --check
```

回报commit/tree/lock、matched test count、v4 fields、combined严格性、full允许集合、失败取证、反搜第二owner与extra-review。
需要越写集、真实build/full或公共语义时停止。
