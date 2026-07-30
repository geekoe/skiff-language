# P5-R22B：Host Result Global Uniqueness Reacceptance

## 角色与范围

只允许提出P5-R22唯一blocker的原reviewer复验同一精确问题；不得把新任务、其它R22矩阵或阶段验收追加给该会话。
输入为P5-R22 FAIL、P5-F22B合同/result、F22B dev/integration commit、root cheap combined `80/80`及包含active I16、
G16E、R23全部前置合同的exact clean candidate。

只给`R22B PASS`或`R22B FAIL`，不修改/提交，不运行Cargo、dependency install、I16、Host/full/source suite/runtime/stable；
不重跑R22已PASS的其它矩阵。PASS只关闭global uniqueness blocker并解锁同一candidate的replacement I16。

## 精确复验

1. diff必须仅为F22B合同允许的Host evidence module与focused test；schema、orchestration、runner/fixture、dependency及lock
   blobs不变。
2. production owner对stdout/stderr全部语法合法、exact-test-name identity先做全局计数；success要求总数1且唯一项位于
   stdout两条result之间。`exactPassLineCount`必须报告全局数。
3. 正确Host目标叠加before-std、after-Host、stderr同名identity三种组合逐项FAIL：count2、observed null、sourceSuite
   null、所有歧义行只留SHA token。
4. alternate合法module作为唯一Host目标仍PASS；不得硬编码runtime module或复制parser。
5. 聚焦测试至少matched上述四项并全PASS；反搜production第二parser和`PASS main.__test::`/
   `PASS main.test.skiff::`均为0。extra-review只针对本次两文件delta。

推荐命令：

```bash
node --test --test-name-pattern 'alternate runtime module|same-name identities outside the Host segment' \
  scripts/tests/platform-source-probe-host-evidence.test.mjs
node --check scripts/lib/platform-source-probe-host-evidence.mjs
git diff --check
```

回报exact candidate/tree/lock、matched/pass/fail、三组合字段、alternate正例、blobs/clean、blocking findings与
extra-review。candidate或Host evidence owner再变化会使R22B失效。
