# P5-F441O Encrypted live harness responsibility split result

状态：`PASS / BEHAVIOR_PRESERVING_RESPONSIBILITY_SPLIT`。

## 1. 输入、提交与写集

- 任务声明 implementation baseline：
  `efee31bce177f865077340dad5aca4a2ec856282`；
- leaf dispatch HEAD：
  `9573b4bb679587258346ebca530bdde3e25c5784`；
- implementation：
  `208e46c335be86561cb301b3e81d1f94ccd3a852`
  （tree `12e77a89c687ae84fba89145806d69ff6202ab80`）。

Implementation只修改任务允许的七个script/test文件：

- `scripts/lib/encrypted-storage-live-harness.mjs`；
- `scripts/lib/encrypted-storage-live-contract.mjs`；
- `scripts/lib/encrypted-storage-live-mongo-probe.mjs`；
- `scripts/lib/encrypted-storage-live-instance-resources.mjs`；
- `scripts/tests/encrypted-storage-live-harness.test.mjs`；
- `scripts/tests/encrypted-storage-live-mongo-probe.test.mjs`；
- `scripts/tests/encrypted-storage-live-instance-resources.test.mjs`。

没有修改checker调用方、source roots、test-runner、verify plan、Router/Runtime、live config、fixture或其它
task/result。本文由独立result-only commit交付。

## 2. Test-first RED

先在direct harness test增加三个owner文件和单向dependency断言，再执行：

```bash
node --test scripts/tests/encrypted-storage-live-harness.test.mjs
```

未拆分实现按预期得到`8 passed / 1 failed`，唯一失败为
`encrypted-storage-live-contract.mjs`不存在（`ENOENT`）。随后才新增owner模块并移动实现。

## 3. 终态责任边界

### 3.1 Contract owner

`encrypted-storage-live-contract.mjs`独占：

- `repoRoot`与`dev` target contract；
- runner/build argv；
- production receipt与canonical assembly校验；
- manifest ingress request；
- caller-owned activation/generation lifecycle；
- 上述契约专用的pure validator/helper。

原harness路径继续re-export原有六个contract symbol。旧路径完整export集合仍精确为十个symbol，没有新增
facade export。

### 3.2 Mongo probe owner

`encrypted-storage-live-mongo-probe.mjs`通过只接收`mongoPort`、`cwd`、command与delay seam的factory
拥有mongosh URL/expression、raw document读写、database/collection发现、replica-set初始化和transient
encrypted envelope解码。harness原有十二个Mongo相关方法均保留原名并delegation。

probe存放在module-private `WeakMap`，没有给class实例增加可枚举公开状态。

### 3.3 Instance resource owner

`encrypted-storage-live-instance-resources.mjs`拥有：

- `45000`–`45999`端口选择、禁区与lease；
- temp instance paths与原config文本；
- PID metadata ownership validation；
- process group TERM、bounded wait、survivor-only KILL。

termination只通过先验证全部metadata的stopper暴露，不存在接收任意PGID并绕过ownership检查的production
primitive。validated callback让harness在任何signal之前记录
`cleanupFallbackUsed`/`cleanupFallbackGroups`；module-private `WeakSet`同时保持直接调用原class方法时的
公开状态行为。factory只返回`{ paths, portLease }`，不构造harness。

### 3.4 Facade

`encrypted-storage-live-harness.mjs`由1076行收敛为534行，继续拥有initialize/build/test/activation、
keyring/restart、HTTP retry、cleanup、command/log orchestration。class prototype方法集合与实例可枚举
状态均有精确回归断言，checker继续只依赖旧import路径。

## 4. Hermetic证据

规定测试：

```bash
node --test \
  scripts/tests/encrypted-storage-live-harness.test.mjs \
  scripts/tests/encrypted-storage-live-mongo-probe.test.mjs \
  scripts/tests/encrypted-storage-live-instance-resources.test.mjs
```

结果：`15 passed / 0 failed`。

覆盖包括：

- 旧harness精确export、prototype method与可枚举state；
- F441K runner/build/receipt/request/lifecycle断言；
- 三owner无harness import、无互相依赖/循环；
- retired-surface扫描harness、三个owner与checker；
- fake mongosh的URL、expression、cwd、读写结果、replica初始化与canned envelope decode；
- fake resource lease/fs的port禁区、paths与config文本；
- ownership mismatch在callback/signal前fail closed；
- validated callback先于TERM，40次bounded wait后只KILL survivor。

额外兼容回归：

```bash
node --test scripts/tests/platform-source-transport-combined.test.mjs
```

结果：`1 passed / 0 failed`，使用fake Cargo，未访问网络或live target。

静态验证：

```bash
node --check scripts/lib/encrypted-storage-live-harness.mjs
node --check scripts/lib/encrypted-storage-live-contract.mjs
node --check scripts/lib/encrypted-storage-live-mongo-probe.mjs
node --check scripts/lib/encrypted-storage-live-instance-resources.mjs
git diff --check
```

全部PASS。

## 5. 隔离与收尾

- 未运行`db-encrypted-storage-live`或任何其它live selector/workload；
- 未启动或访问Mongo、instance、server、Router、Runtime、telemetry、watch或网络；
- 未派sub-agent，未merge、rebase或push；
- 三个owner保持从harness出发的单向依赖，没有触发`TASK_SCOPE_EXPANDED`。
