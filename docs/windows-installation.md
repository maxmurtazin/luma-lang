# Windows x64 installation

Download the Windows installation distribution from the successful **Windows installation distribution** GitHub Actions run on main (signed-in GitHub account required for Actions artifacts), or from a tagged GitHub Release when available. Extract the artifact ZIP and run `Luma-0.1.2-windows-x64-setup.exe`. Verify SHA256SUMS.txt with `Get-FileHash`. This installer is unsigned: Windows SmartScreen may display a warning. No code-signing certificate is configured.

The installer runs as the current user without administrator rights. It installs the native Rust CLI, bundled VSIX, documentation and extension setup helper in `%LOCALAPPDATA%\Programs\Luma`. Rust, Node.js and build tools are not needed on the user's machine. Windows x64 is supported.

The checked **Install Graph Explorer extension in VS Code** task locates stable VS Code in its per-user or system location, then searches PATH for code.cmd. It installs the bundled VSIX with `--force` in the default VS Code profile. Close VS Code before upgrading and reload it after installation. Other profiles, portable VS Code, Insiders, WSL and remote hosts require manual extension installation and an appropriate runtime in that environment.

If VS Code is absent or extension setup fails, the CLI remains usable and the VSIX stays in the install folder. Read `extension-install.log`, install VS Code 1.90+, then use **Extensions: Install from VSIX** and select the bundled VSIX. No network is needed to install the bundled files. The extension helper can also be rerun with PowerShell using `-File Install-Extension.ps1 -InstallDir <install-folder>`.

## Interpreter discovery and commands

The extension's default `luma.executablePath` value (`luma`) first checks the standard Windows install folder, so even an already-running VS Code can find the newly installed interpreter. Explicit custom values take priority; remove a stale custom value or set it to the installed executable. The installer does not edit VS Code settings or PATH. To use a short `luma` command in terminals, optionally add `%LOCALAPPDATA%\Programs\Luma\bin` through Windows Environment Variables, then open a new terminal. Full paths always work:

```powershell
& "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" version
& "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" run .\main.luma
& "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" check .\main.luma
& "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" check --graph .\graph.json .\main.luma
```

Save a local `.luma` file, trust its workspace, then select **Luma: Run File**, **Luma: Check File**, or **Luma: Open Graph Explorer** from the command palette. Graph nodes navigate to source. Save imported files before checking. CLI exit codes: 0 success, 1 check/semantic failure, 2 usage error, 3 runtime failure.

## Options, upgrades and uninstall

The install directory is fixed for reliable discovery. Uncheck the extension task to install just the CLI and retain the bundled VSIX for later. For unattended installation use `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`; add `/TASKS=""` to skip VS Code installation. Unattended users should inspect extension-install.log for extension failures.

Upgrades use a stable application ID and replace only packaged files. Back up any modified bundled files; store your projects elsewhere. The extension task uses `--force` and can replace an existing extension version, including a newer one: do not use an older installer to downgrade. Close running Luma processes and VS Code before upgrading. Uninstall through Windows Installed Apps or unins000.exe. It removes packaged files but preserves unrelated user files, VS Code settings, projects, and the installed extension. Remove the extension separately using VS Code Extensions if desired. Any PATH entry you added manually also needs manual removal. The retained extension needs a runtime to run commands.

## Build and validation

On a Windows x64 build machine with Rust's MSVC toolchain, Visual Studio C++ build tools, Node.js 22, Inno Setup 6 and VS Code:

```powershell
cargo test --locked --target x86_64-pc-windows-msvc
cargo build --release --locked --target x86_64-pc-windows-msvc
cd editors/vscode
npm ci
npm test
npm run package
cd ../..
& "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" packaging/windows/luma.iss
# Only in a disposable user account:
./packaging/windows/Smoke-Test.ps1
```

CI builds on windows-2022, tests Rust and extension units, compiles the installer, installs twice, checks preservation of unrelated files, invokes the installed Run/Check/Graph CLI, installs the bundled VSIX into isolated VS Code directories, and uninstalls. VSIX installation is not an Extension Host UI test. Manually verify command palette Run/Check/Graph, source navigation, custom executable settings, missing VS Code, and SmartScreen on a clean Windows account before a public release. All six 0.1D2 standard library operations execute.

Current guides: [English](https://maxmurtazin.github.io/luma-lang/en/docs/windows/) · [Русский](https://maxmurtazin.github.io/luma-lang/ru/docs/windows/).
