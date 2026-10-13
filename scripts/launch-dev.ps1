param([switch]$Rebuild)

$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$targetDir = if ($env:CARGO_TARGET_DIR) {
    if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) { $env:CARGO_TARGET_DIR }
    else { Join-Path $repoRoot $env:CARGO_TARGET_DIR }
} else { Join-Path $repoRoot 'target' }
$app = Join-Path $targetDir 'debug\koharu.exe'
$server = $null

Push-Location -LiteralPath $repoRoot
try {
    $bun = (Get-Command bun -ErrorAction Stop).Source
    if (-not (Test-Path -LiteralPath 'node_modules')) {
        throw 'Install dependencies first: bun install --frozen-lockfile --ignore-scripts'
    }
    if ($Rebuild) {
        & $bun run --filter @koharu/bridge build:dev
        if ($LASTEXITCODE -ne 0) { throw 'The debug WebAssembly bridge build failed.' }
        & cargo run --locked -p koharu-app --bin generate
        if ($LASTEXITCODE -ne 0) { throw 'Bridge protocol generation failed.' }
        & cargo build --locked -p koharu --bin koharu
        if ($LASTEXITCODE -ne 0) { throw 'The native debug build failed.' }
    }
    if (-not (Test-Path -LiteralPath $app)) {
        throw 'No debug build exists yet. Choose option 2 once to build it.'
    }
    if (-not (Test-Path -LiteralPath 'packages\bridge\src\wasm\koharu_canvas.js')) {
        throw 'The canvas bridge is missing. Choose option 2 once to build it.'
    }
    if (Get-NetTCPConnection -LocalPort 3000 -State Listen -ErrorAction SilentlyContinue) {
        throw 'Port 3000 is in use. Close the other dev session before launching this checkout.'
    }

    $logDir = Join-Path $repoRoot '.local-git\dev'
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
    $server = Start-Process -FilePath $bun -ArgumentList @(
        'run', '--parallel', '--filter', '@koharu/bridge', '--filter', '@koharu/app', 'dev'
    ) -WorkingDirectory $repoRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $logDir 'frontend.log') `
        -RedirectStandardError (Join-Path $logDir 'frontend-error.log')

    Write-Output 'Starting the frontend with hot reload. Logs: .local-git\dev'
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $ready = $false
    while ([DateTime]::UtcNow -lt $deadline -and -not $server.HasExited) {
        try {
            $null = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:3000' -TimeoutSec 2
            $ready = $true
            break
        } catch { Start-Sleep -Milliseconds 250 }
    }
    if (-not $ready) { throw 'The frontend did not become ready. Check .local-git\dev logs.' }

    Write-Output 'Launching the saved debug build. UI changes reload automatically.'
    Write-Output 'After Rust changes, close the app and use option 2. Debug inspection: http://127.0.0.1:4000'
    & $app
    if ($LASTEXITCODE -ne 0) { throw "Koharu exited with code $LASTEXITCODE." }
}
catch {
    Write-Error $_ -ErrorAction Continue
    exit 1
}
finally {
    if ($server -and -not $server.HasExited) {
        & taskkill.exe /PID $server.Id /T /F | Out-Null
    }
    Pop-Location
}
