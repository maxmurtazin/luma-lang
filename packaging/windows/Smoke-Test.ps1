# Run only in a disposable Windows CI account: uses the standard per-user install path.
$ErrorActionPreference = 'Stop'
$installer = (Get-ChildItem dist/*-setup.exe | Select-Object -First 1).FullName
$app = Join-Path $env:LOCALAPPDATA 'Programs\Luma'
if (Test-Path $app) { throw 'Refusing to test over an existing Luma installation' }
function Run-Setup {
    $p = Start-Process -FilePath $installer -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TASKS=""' -WindowStyle Hidden -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "Setup failed: $($p.ExitCode)" }
}
try {
    Run-Setup
    Set-Content "$app\user-marker.txt" 'preserve on upgrade'
    Run-Setup
    if (!(Test-Path "$app\user-marker.txt")) { throw 'Upgrade removed user data' }
    $cli = "$app\bin\luma.exe"
    $sample = "$env:RUNNER_TEMP\stdlib.luma"
    Set-Content $sample "fn square(x)
return x*x
end
print([1,2,3] -> parallel(square) -> sum)"
    $result = & $cli run $sample
    if ($LASTEXITCODE -or $result -ne '14') { throw 'Installed stdlib failed' }
    & $cli version
    if ($LASTEXITCODE) { throw 'Version failed' }
    & $cli run editors/vscode/test/fixtures/main.luma
    if ($LASTEXITCODE) { throw 'Run failed' }
    & $cli check --graph "$env:RUNNER_TEMP\luma-graph.json" editors/vscode/test/fixtures/main.luma
    if ($LASTEXITCODE) { throw 'Check failed' }
    $graph = Get-Content "$env:RUNNER_TEMP\luma-graph.json" -Raw | ConvertFrom-Json
    if ($graph.schemaVersion -ne 1 -or !$graph.valid) { throw 'Invalid graph' }
    $code = Get-Command code.cmd -ErrorAction Stop
    $userDir = "$env:RUNNER_TEMP\luma-vscode-user"
    $extDir = "$env:RUNNER_TEMP\luma-vscode-extensions"
    & $code.Source --user-data-dir $userDir --extensions-dir $extDir --install-extension "$app\luma-language-tools-0.1.2.vsix" --force
    if ($LASTEXITCODE) { throw 'VSIX installation failed' }
    $extensions = & $code.Source --user-data-dir $userDir --extensions-dir $extDir --list-extensions
    if ($extensions -notcontains 'maxmurtazin.luma-language-tools') { throw 'Extension missing' }
} finally {
    if (Test-Path "$app\unins000.exe") {
        $p = Start-Process "$app\unins000.exe" -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -Wait -PassThru
        if ($p.ExitCode -ne 0) { throw 'Uninstall failed' }
        if (Test-Path "$app\bin\luma.exe") { throw 'CLI survived uninstall' }
        if (!(Test-Path "$app\user-marker.txt")) { throw 'Uninstall removed unrelated user data' }
    }
}
