# P5-F27B：Canonical Std Seed / Bootstrap

依赖F27A exact commit合流。独占test-runner canonical store/bootstrap helper、package-service smoke fixture binary与直接tests；
不回改compiler writer、scripts、runtime/Router。一个clean commit。

实现`seed_canonical_std(platform_sources, artifact_root)`：调用F27A official authoring与唯一writer；任何写前验证现有pointer，
same exact candidate幂等，missing pointer用CAS None→candidate，并发loser重读同值成功/异值冲突；malformed/dangling/different
pointer和same-ID/different-bytes均fail closed。删除test-runner `storage_canonical_package`字节规范owner。

`--bootstrap-only`与fresh-store exact regression复用同一helper；bootstrap receipt包含exact std ref/build/record/pointer，Cargo
调用使用`--locked`。compile仍必须从store resolve std，不得隐式读platform source。tests覆盖顺序/并发重复、crash-safe orphan
record、negative pointer及D38C exact regression完成production→overlay→2 deployments→3-package assembly；保留missing-std
compiler负例。禁止Router/runtime/activation/top-level smoke/full/I16/Host/stable。
