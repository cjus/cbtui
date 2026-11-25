# The pieces

The twelve chess pieces cbtui draws on terminals that can show images: Colin M. L. Burnett's
set, the one Wikipedia's diagrams use.

They are Wikimedia Commons' `Chess_{k,q,r,b,n,p}{l,d}t45.svg` (`l` White, `d` Black), renamed
by side and piece (`wK.svg` is White's king) and otherwise unchanged. Each file's Commons page
offers it under the GFDL, the BSD licence or the GPL; cbtui takes them under the BSD licence,
3-clause (`LICENSE.txt`), which is permissive and fits cbtui's `MIT OR Apache-2.0`. Some sites
redistribute the same set under the GPL only: take them from Commons.

cbtui builds them into its binary and draws them at the size of a square (`src/pieces.rs`).
Their SHA-256s:

```
ba67da76ce919addc60ecb8b46801def073dd54149b2c038a2d07a16d904d5e4  bB.svg
025eea92e0ef8eb1fd06b1c58d0d112948f08bf66cea6b5d003659569949b41c  bK.svg
735cc58315b123a56632d4877a6b976c827481fa97bf9a5c8f459ec969bc2549  bN.svg
4413bf7c18a341f9723d97e6f92c985e30b6167b037e80842cea59b7541bb074  bP.svg
70191a3fbc729ef629661e2419a66ab8024c49277aab8ccae3a5ef61372ab802  bQ.svg
6abf617a9e26902e0734d85897c9ca55e29d7be2928142aa21032c38967e34ba  bR.svg
1d7beace24d455c923ee80d27125963eaf0287b956c5576dbf790c97ac0b97eb  wB.svg
56f55c784843b1ac272b8745d740aa2a3e6c585513ef889978916f88e5d0b70b  wK.svg
5486791207156f7ae8b8678187648df45085d726334c2862e73b077dea00641e  wN.svg
cc7de30708dcec8f4d593a89d10893d5f9c063682039a1c441e86c44cf2096db  wP.svg
b72b864e2a5b6c8f8afb7f260130c10e649ff063f4ef58190c00a35c56364327  wQ.svg
4d42ab45afd862c704eb9b35317102d453a7a6b9b71d40f18958c8eadc829e4b  wR.svg
```
