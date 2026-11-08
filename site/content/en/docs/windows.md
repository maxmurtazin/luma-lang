---
title: "Windows installation"
weight: 2
---

Setup installs per user into %LOCALAPPDATA%\Programs\Luma without administrator rights or PATH changes. Run & "$env:LOCALAPPDATA\Programs\Luma\bin\luma.exe" version in PowerShell. Use that full path for check/run, or configure luma.executablePath in VS Code. The default extension setting discovers this location.

Select the optional VS Code task to install the bundled VSIX. Reload VS Code afterwards. If VS Code is missing or installation fails, read extension-install.log and use Extensions: Install from VSIX on luma-language-tools-0.1.2.vsix in the install folder. The standalone VSIX does not contain the interpreter.

Rerun setup to upgrade in place. Close running Luma processes first. Uninstall through Windows Installed apps. Only installer-owned files are removed; user files and the separately registered VS Code extension remain. Remove that extension through VS Code if desired. CI checks a repeated installation, user-file preservation, CLI, graph export, VSIX registration and uninstall in a disposable Windows account. A repeated installation is not a test of migration from an older release.
