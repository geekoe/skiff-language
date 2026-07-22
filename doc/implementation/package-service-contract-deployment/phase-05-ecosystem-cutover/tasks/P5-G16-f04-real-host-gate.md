# P5-G16：F04 Real Host Gate

## 输入与唯一owner

使用未参与F16A/B/C/F17/F18A–J/F19A/B实现、D20/D25审计/修复、I16或R16的全新Gate Agent。输入为repair wave全部合流、I16
combined PASS、R16 PASS且三者锚定同一exact clean candidate/tree/lock；不得编辑、提交、修复或操作stable。权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14及阶段标准2/4/5/6。

`10746a2`上的第一次full-mode调用在Host前因artifact comparator失败；`f82282c`上的第二次artifact PASS、Host child
code1，但v4丢失bounded diagnostic。D25/D26及F19/F20/P26S关闭后，新candidate的Gate是当前周期第三次且唯一剩余
full-mode调用。它不得重试；若失败不允许第四次，必须建立新的审计/combined周期并向用户说明。

## 唯一命令与冻结行为

Gate Agent核对clean candidate、combined ledger的commit/tree/lock/golden/cleanup及R16 PASS，然后从非repo cwd只执行：

```bash
P5_G16_ROOT=/Users/geek/workspace/skiff-phase-05-integration
P5_G16_COMMIT="$(git -C "$P5_G16_ROOT" rev-parse HEAD)"
P5_G16_TREE="$(git -C "$P5_G16_ROOT" rev-parse HEAD^{tree})"
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-platform-source-shared-target-probe.mjs \
  --mode full \
  --integration-root "$P5_G16_ROOT" \
  --candidate "$P5_G16_COMMIT" \
  --expected-tree "$P5_G16_TREE" \
  --expected-lock-blob f3ce5457138c58aec4c84abda431afa96013e3fd \
  --expected-prelude-identity skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d \
  --expected-std-package-build-id skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4 \
  --combined-ledger /Users/geek/workspace/skiff-phase-05-integration/.p5-i16-combined-ledger.json \
  --a-worktree /Users/geek/workspace/skiff-p5-g16-a \
  --b-worktree /Users/geek/workspace/skiff-p5-g16-b \
  --json
```

`--mode full`必须拒绝candidate/tree/lock/golden或v5 combined ledger不一致；不得重跑merge-only fixture、local test或完整
combined矩阵。它只建立复用同类shared target的A-origin artifacts，验证B-root消费仍为Fresh，然后从任务临时目录调用
B的absolute `scripts/run-skiff-tests.mjs`一次。任何primary失败立即停止且不自动重试；cleanup secondary单独保留。

## PASS与交付

PASS必须同时记录：std exact `11/11`、Host exact `1/1`、最终值`provider-observed-helper-mutated`、A-origin/B-root Fresh
证据、candidate/tree/lock、完整probe run count恰为1，以及worktree/temp/PID/port cleanup ABSENT。PASS才解除新的F04
receive reviewer；candidate不变时reviewer直接消费G16 ledger，不得再次运行Host。若失败，回到D20汇总出的repair DAG；
不得在Gate会话内修复。
