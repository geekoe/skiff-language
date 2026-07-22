# P5-R16：Platform Source Trust Acceptance

未参与F16A/B/C/F17实现、D19审计或I16 gate的新的独立只读Agent。输入为四任务合流后的exact clean commit/tree、
D18/F16A/B/C/F17合同与I16唯一
动态ledger；不得编辑、提交、修复、操作stable或重跑完整source-suite/Host，也不作F04/R02总体verdict。
I16 ledger还必须证明D19 `DESIGN GO`已由F17 exact repair关闭且无在途写入。
权威设计为`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14及阶段标准6。

第一行只给`R16 PASS`或`R16 FAIL`。必验：

- F16A是唯一runtime platform trust owner；absolute canonical validation、registry containment、official provenance、
  same-root idempotence与different-root fail-closed符合合同；reserved id没有放宽。
- compiler library、binary authoring、runner、smoke fixture、source-suite、`skiff test`全部显式消费同一个root；无
  cwd/env/executable/`CARGO_MANIFEST_DIR` production fallback、第二helper、dual path或clean-cache依赖。只允许ignored
  Rust identity probe读取`SKIFF_TEST_PLATFORM_SOURCE_ROOT`；`__ecosystem-store`无源码action不构造context。
- I16确实以A-built/Fresh-B、不同worktree路径和共享target跑过std 11/11 + Host exact 1/1；候选、binary hash/mtime、
  lock和环境证据一致，四次probe的8个带标签identity均等于D18 golden、legacy identity bit-identical，关键负例已关闭，
  资源已清理。
- F17真实FileHandle/child交错证据PASS，supervisor无fire-and-forget close/立即exit/第二lifecycle owner。
- production artifact/dep-info无worktree platform常量；source registry不扩张，fake reserved package、missing/cross-root、
  relative/omitted/context mismatch均fail closed，canonical symlink正例成立。
- 未修改Router/Runtime/schema/fixture业务语义/manifest/lock；直接触碰的大文件职责收敛，运行`extra-review`，不把
  行数本身当finding。

只运行风险所需的聚焦抽查，不重复开发者仍有效测试或I16完整gate。PASS解除F04 narrow receive；若exact候选未变，
F04必须消费I16 Host证据而非再跑一次。回报blocking issues、non-blocking follow-up、命令、动态缺口和残余风险。
