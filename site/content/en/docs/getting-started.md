---
title: "Getting started"
aliases: ["/docs/getting-started/"]
weight: 1
---

Luma 0.1.3 packages the 0.1D2 bounded interpreter. Install the Windows x64 bundle from GitHub Releases or the Windows installation distribution Actions artifact. Unzip the artifact before running setup. It includes the CLI and VSIX; VS Code 1.90+ is installed separately. Save UTF-8 source as hello.luma.

From a source checkout: install stable Rust with the platform C/C++ linker, then run cargo build --release --locked. On Windows use Visual Studio Build Tools with Desktop development with C++. Run target/release/luma.exe; on Linux/macOS use target/release/luma.

~~~text
print("Hello, Luma")
~~~

**Expected output:**

~~~text
Hello, Luma
~~~
