cbtui is a terminal chess client. Each download is cbtui and the cb2026 engine it plays,
ready to run: unpack it and start `cbtui` in a terminal.

| Computer | Download |
| --- | --- |
| Mac with Apple silicon (M1 and later) | `cbtui-…-aarch64-apple-darwin.tar.gz` |
| Mac with an Intel processor | `cbtui-…-x86_64-apple-darwin.tar.gz` |
| Linux, x86_64 | `cbtui-…-x86_64-unknown-linux-gnu.tar.gz` |
| Linux, ARM64 | `cbtui-…-aarch64-unknown-linux-gnu.tar.gz` |
| Windows | `cbtui-…-x86_64-pc-windows-msvc.zip` |

**macOS and Linux**

```sh
tar xzf cbtui-*.tar.gz
cd cbtui-*/
./cbtui
```

**Windows**: unzip it, open the folder in Windows Terminal, and run `.\cbtui.exe`.

cbtui finds the cb2026 beside it. To run it from anywhere, put both on your `PATH`. Press `?`
in cbtui for help; `cbtui --help` lists the options.

The macOS programs are signed and notarised, so macOS checks them with Apple the first time
they run, which needs a network connection.

Both programs are `MIT OR Apache-2.0`. The cb2026 here is built without Syzygy tablebase
probing; the engine's own releases, at https://github.com/cjus/chessengine/releases, have it.
`THIRD-PARTY-NOTICES.txt` lists the crates each is built with and their licences.
`SHA256SUMS.txt` has each download's checksum.
