# P5-I03：Cross-Repo Generated RuntimeAssembly Combined Probe

## 角色与输入

主integration owner在T06、T07、T09E分别合入三个integration branch、工作树clean且无在途写入后执行；
不是开发、gate或独立验收owner。冻结三个exact commits/trees，使用最终Skiff integration、
`skiff-packages` integration与Internals integration；不得操作stable或修改文件。

## 唯一命令与完成态

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-i03-cargo.XXXXXX)"
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  node /Users/geek/workspace/internals-phase-05-integration/scripts/run-phase05-ecosystem-probe.mjs \
  --isolated --replicas 1
git -C /Users/geek/workspace/skiff-phase-05-integration diff --check
git -C /Users/geek/workspace/skiff-packages-phase-05-integration diff --check
git -C /Users/geek/workspace/internals-phase-05-integration diff --check
```

脚本必须在temporary store/runtime/router/Mongo与fake upstream上，从命令显式给出的真实五个service
source roots和`skiff-packages` source独立compile packages、validate deployments、收集exact deployment
receipts、resolve one generated RuntimeAssembly并完成activation transaction。不得读取repo-level
`assembly.yml`。至少断言account/registry ping、Codex/AIHub同path不同Host、
Agine provider/list含`aihub/gpt-5.5`及一条chat最终结果；legacy selector不能改变target，tampered candidate
abort后旧generation仍服务。不得调用AIHub/Agine build/dev/start或stable reload。

输出三仓exact commit/tree、root来源及receipts、四对象closure、activation/replica provenance、每个Host
最终结果、负例与耗时。
PASS才可提交R03；T13不得重跑本命令。FAIL退回精确owner，修复合流后重跑本probe与失效证据。
