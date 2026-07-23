# P5-F56C2：Host Trusted Registry Wiring

基于F56C0接入Host/eval native：request-scoped `Arc<dyn TrustedRegistryStoreApi>`、deployment principal、
operation scope、deadline/cancel；只有exact `skiff.registry.trusted@1` binding与allowlisted registry principal
获得context。普通package、伪serviceId、缺/错capability、越权scope、retired/cancelled request fail closed。

可用trait double，不依赖C1 persistence；不改Router activation。运行native/host正负测试、check/rustfmt/diff，
单commit；禁止stable/full gate。
