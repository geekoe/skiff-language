# P5-F18E：Supervisor Process Lifecycle Closure

权威设计：`doc/architecture/package-service-contract-deployment.md` §11、§14；D19/F17与D20 result。从D20 docs
checkpoint建立`/Users/geek/workspace/skiff-p5-f18e-supervisor-lifecycle`、
`codex/p5-f18e-supervisor-lifecycle`。全新Agent、一个commit，不merge/push/stable/Router/Runtime；五分钟内修改。

exclusive write set：`scripts/lib/supervised-entry-lifecycle.mjs`、`scripts/skiff-instance.mjs`、可新增单一
`managed-pid-metadata.mjs`及对应supervisor/PID tests。禁止触碰isolated/gate/compiler/test-runner、manifest/lock。

完成态：SIGKILL后仍alive或`{stopped:false}`必须reject；未证group absent时保留PID、禁止restart，但两个handles仍
all-settled关闭。spawn后立即由唯一lifecycle接管；unsupervised PID write/health/任一close失败也经同一stop/completion，
首个close失败不得跳过第二个。PID metadata用nonce+inode+no-clobber安装/条件删除；foreign/pre-existing/replacement
一律保留并阻止restart。primary error先于cleanup，strict unhandled rejection下无泄漏。

```bash
node --unhandled-rejections=strict --test scripts/tests/skiff-instance-supervisor-lifecycle.test.mjs scripts/tests/skiff-instance-pid-metadata.test.mjs
node --test scripts/tests/skiff-instance-config.test.mjs
node --check scripts/lib/supervised-entry-lifecycle.mjs
node --check scripts/lib/managed-pid-metadata.mjs
node --check scripts/skiff-instance.mjs
git diff --check
```

保留F17 20轮并新增false-stop、PID write、单边close、replacement/no-clobber/restart negatives。PID owner必须从1738行入口
提取；不得新增第二child lifecycle。回报commit/tree/lock、FD/PID/process absence、error order、extra-review。
