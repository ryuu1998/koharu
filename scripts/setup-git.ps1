$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))

Push-Location -LiteralPath $repoRoot
try {
    $gitRoot = & git rev-parse --show-toplevel
    if ($LASTEXITCODE -ne 0 -or [IO.Path]::GetFullPath($gitRoot) -ne $repoRoot) {
        throw 'Run this script from a clone of the Koharu repository.'
    }

    $settings = [ordered]@{
        'core.hooksPath' = '.githooks'
        'core.autocrlf' = 'false'
        'core.longpaths' = 'true'
        'pull.ff' = 'only'
        'fetch.prune' = 'true'
        'push.default' = 'simple'
        'remote.pushDefault' = 'origin'
        'merge.conflictStyle' = 'zdiff3'
        'rerere.enabled' = 'true'
        'alias.st' = 'status --short --branch'
        'alias.graph' = 'log --graph --decorate --oneline -20'
        'alias.unstage' = 'restore --staged'
    }
    foreach ($entry in $settings.GetEnumerator()) {
        & git config --local $entry.Key $entry.Value
        if ($LASTEXITCODE -ne 0) { throw "Could not set $($entry.Key)." }
    }

    Write-Output 'Git workflow installed for this checkout. See GIT_WORKFLOW.md.'
}
finally {
    Pop-Location
}
