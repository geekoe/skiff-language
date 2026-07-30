# P5-F22B：Host Result Global Uniqueness

## 输入、owner与边界

输入为P5-R22在`9de99ab547da68813235a3de97c3772f201484ad`上唯一FAIL。权威业务语义仍是P5-F04/F04A的
唯一checked-in Host test、唯一assertion与`provider-observed-helper-mutated`；module path和ledger字段不是公共契约。

使用全新开发Agent，从包含本合同与R22 FAIL result的exact integration checkpoint创建`/Users/geek/workspace`下独立
worktree/branch；一个clean commit后结束，不merge/push/stable。exclusive write set仅：

- `scripts/lib/platform-source-probe-host-evidence.mjs`
- `scripts/tests/platform-source-probe-host-evidence.test.mjs`

不得修改evidence re-export、shared-target orchestration/tests、diagnostic/contract/schema、runner/discovery/fixture/source suite、
dependency/Router/Runtime、manifest/lock或设计文档。若修复必须越界或改变公共语义，停止报告。

## 完成态

1. production owner先对stdout和stderr的全部PASS行应用现有512-byte/ASCII identity grammar，统计所有语法合法且test name
   精确为`provider observes helper mutation`的identity；不得先按segment过滤后再声称唯一。
2. success必须同时满足：全输出exact-name identity总数恰好1；该唯一项来自stdout；其index严格位于第一条exact std
   result之后、第二条exact Host result之前。`exactPassLineCount`记录全输出exact-name总数。
3. Host段目标 + std前、第二result后或stderr同名identity三种组合全部FAIL，`sourceSuite:null`；missing/wrong/Host段
   duplicate及原有malformed/oversized矩阵继续fail closed。alternate合法module作为全输出唯一Host目标继续PASS。
4. `observedPassLine`只在“全局唯一且位置正确”时保存actual；发生任何同名歧义时不得选择一条冒充观察值。`passLines`
   对歧义/非目标行只保留既有SHA-256 token，不泄漏raw；唯一合法目标成功时才保留actual完整行。
5. 11/11→1/1、command/process/port、fixture assertion/finalValue、v6 diagnostics、Host attempt字段集合及module-agnostic parser
   全部不变；schema保持v6，不新增字段、不硬编码`main.__test`。
6. success parser仍只有该文件一个owner；测试直接调用production owner，不复制grammar/segment/parser。

## 聚焦验证与交付

禁止Cargo、dependency install、I16、Host/full/source suite/runtime/stable。至少运行：

```bash
node --test scripts/tests/platform-source-probe-host-evidence.test.mjs
node --check scripts/lib/platform-source-probe-host-evidence.mjs
git diff --check
```

要求新增三个组合负例逐项matched，并回报总pass/fail、global-count/segment/storage矩阵、反搜第二parser/module literal、
commit/tree/lock、clean与extra-review。合流后root执行一次cheap combined，再只允许原R22 reviewer复验其同一精确blocker；
其它验收面不重跑。F22B使当前F22A cheap combined、R22 FAIL候选及未执行的I16/G16E候选身份失效，但不使P27R/R21C
dependency/startup证据失效。
