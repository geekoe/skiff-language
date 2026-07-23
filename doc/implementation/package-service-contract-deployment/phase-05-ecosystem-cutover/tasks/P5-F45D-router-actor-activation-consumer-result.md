# P5-F45D：Router Actor Activation Consumer Result

结论：COMPLETE。

- task commit：`94c346031b4c67ecc426a38b63192bda2321ef77`
- integration commit：`b1fd7534c5f31c5c4924476d3c79ce7c278d8a1d`
- integration tree：`46a7f4736b74948950fca4c334ee1ab95c1d666e`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

Router现只接受exact assembly connection structured identity；active与pinned-draining允许，drained及任一tuple mismatch
拒绝。legacy/package-test fallback已删除，spawn queue保留完整tuple。Router三文件58项及type-check PASS，F45B的12个
consumer errors全部关闭。
