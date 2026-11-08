param([Parameter(Mandatory=$true)][string]$InstallDir)
$ErrorActionPreference = 'Stop'
$log = Join-Path $InstallDir 'extension-install.log'
try {
    $candidates = @(
        "$env:LOCALAPPDATA\Programs\Microsoft VS Code\bin\code.cmd",
        "$env:ProgramFiles\Microsoft VS Code\bin\code.cmd"
    )
    $found = Get-Command code.cmd -ErrorAction SilentlyContinue
    if ($found) { $candidates += $found.Source }
    $cli = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (!$cli) { throw 'VS Code was not found. Install VS Code 1.90+ and use Extensions: Install from VSIX with the bundled VSIX.' }
    $vsix = Join-Path $InstallDir 'luma-language-tools-0.1.1.vsix'
    & $cli --install-extension $vsix --force 2>&1 | Out-File -LiteralPath $log
    if ($LASTEXITCODE -ne 0) { throw "VS Code extension installation returned $LASTEXITCODE. See log and install the VSIX manually." }
    Add-Content -LiteralPath $log 'Extension installed. Reload VS Code. Explicit luma.executablePath settings take priority.'
} catch {
    Add-Content -LiteralPath $log "Extension setup incomplete: $_"
    # The interpreter and VSIX remain installed for offline/manual setup.
    exit 0
}
