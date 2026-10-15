$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

Push-Location -LiteralPath $repoRoot
try {
    & git rev-parse --verify HEAD | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'The repository needs its first commit before backup.' }

    $backupDir = Join-Path $repoRoot '.git-backups'
    New-Item -ItemType Directory -Path $backupDir -Force | Out-Null
    $backupFile = Join-Path $backupDir ("koharu-{0}.bundle" -f (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
    if (Test-Path -LiteralPath $backupFile) { throw 'Backup already exists; try again.' }

    & git bundle create $backupFile --all
    if ($LASTEXITCODE -ne 0) { throw 'Git backup failed.' }
    & git bundle verify $backupFile
    if ($LASTEXITCODE -ne 0) { throw 'Git backup verification failed.' }

    Write-Output "Backup: $backupFile"
    Write-Output 'Includes committed history and refs. Commit or stash unfinished work first.'
}
finally {
    Pop-Location
}
