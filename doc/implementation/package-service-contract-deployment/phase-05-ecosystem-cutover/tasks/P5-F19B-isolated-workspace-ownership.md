# P5-F19B：Isolated Workspace Ownership

## 输入与 owner

权威设计：架构§6.1、§6.2、§11、§14；F17/F18E/F与D25 result。使用全新开发Agent，从D25 docs checkpoint建立
`/Users/geek/workspace/skiff-p5-f19b-isolated-ownership`、`codex/p5-f19b-isolated-ownership`。一个clean commit；不merge/
push/stable，不运行combined/I16/H18/full/Host或真实isolated runtime。

exclusive write set：`scripts/lib/isolated-test-runtime.mjs`、`scripts/lib/isolated-test-runtime-instance.mjs`、对应
`scripts/tests/{isolated-test-runtime,test-runner-runtime-isolation}.test.mjs`；可新增一个专用workspace ownership child
module/test。不得改gate harness、source suite、Router/Runtime、instance CLI、fixture、manifest/lock或外层F18F owner。

## 完成态

- `mkdtemp`后立即用exclusive marker、128-bit nonce与dev+ino捕获唯一workspace owner；config写入后也捕获exact identity。
  lifecycle stack始终携带typed ownership receipt，不只保存path。
- stop supervisor仍优先处理内存中owned child；调用`down/status <config>`前必须再次验证workspace marker/path identity与
  config identity。任一foreign replacement、symlink、missing/corrupt marker、inode变化均不得调用foreign config。
- recursive remove前再次验证同一owner；不使用`force`掩盖replacement。ownership mismatch保留foreign path并返回
  cleanup secondary，仍关闭owned child、ports与自身lease；primary-first与all-settled语义不变。
- startup partial、normal teardown、signal/false-stop与双错误路径共享同一owner，不新增第二cleanup实现。12个既有foreign
  `skiff-test-runtime-*`不得枚举或删除。
- 便宜测试在teardown各跳点替换root/config/marker/symlink，断言不调用foreign down/status、不递归删除且receipt可诊断；
  同组审计port lease token replacement。若实际复现lease read→unlink race且需改`local-port-lease.mjs`，停止并报告新的
  独立owner，不越界顺手修。

## 验证与交付

```bash
node --test scripts/tests/isolated-test-runtime.test.mjs
node --test scripts/tests/test-runner-runtime-isolation.test.mjs
node --check scripts/lib/isolated-test-runtime.mjs
node --check scripts/lib/isolated-test-runtime-instance.mjs
git diff --check
```

不得启动真实进程/端口；command doubles必须报告matched>0。回报commit/tree/lock、ownership receipt、foreign preserved、
all-settled/primary-first、port lease结论、第二owner反搜与extra-review。需越写集或改公共语义时停止。
