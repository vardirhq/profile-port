# End-to-end smoke test of the ProfilePort CLI on real Windows.
#
# Builds a test profile containing the things that make Windows migrations
# hard (see docs/windows-gotchas.md), exports it, restores it to a different
# username and checks the result.
#
#   PASS  works
#   FAIL  regression: the script exits non-zero
#   GAP   known, not implemented yet: reported, does not fail the run
#
# Usage: ./scripts/windows-smoke.ps1 -Exe target/release/profileport.exe

param([Parameter(Mandatory = $true)] [string] $Exe)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
$results = New-Object System.Collections.Generic.List[object]

function Check([bool] $ok, [string] $what) {
    $results.Add([pscustomobject]@{ Result = $(if ($ok) { 'PASS' } else { 'FAIL' }); Check = $what })
    Write-Host ("  {0,-5} {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $what)
}

function Gap([bool] $ok, [string] $what) {
    $results.Add([pscustomobject]@{ Result = $(if ($ok) { 'PASS' } else { 'GAP' }); Check = $what })
    Write-Host ("  {0,-5} {1}" -f $(if ($ok) { 'PASS' } else { 'GAP' }), $what)
}

function Mask([string] $text) {
    return [regex]::Replace($text, '[0-9A-Z]{4}-[0-9A-Z]{4}-[0-9A-Z]{4}', '<CODE>')
}

$base = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$root = Join-Path $base 'pp-smoke'
if (Test-Path $root) { Remove-Item $root -Recurse -Force }
$src = Join-Path $root 'Users\anna'
$drive = Join-Path $root 'E'
$dst = Join-Path $root 'Users\anna.smith'
foreach ($d in @("$src\Documents", "$src\AppData\Roaming\App", "$src\AppData\Local\Temp", $drive, $dst)) {
    [IO.Directory]::CreateDirectory($d) | Out-Null
}

Write-Host '== Building test profile'
[IO.File]::WriteAllText("$src\Documents\report.txt", 'quarterly numbers')
[IO.File]::WriteAllText("$src\Documents\Ærø – 日本.txt", 'unicode')
[IO.File]::WriteAllText("$src\AppData\Roaming\App\settings.ini", '[ui]')
[IO.File]::WriteAllText("$src\AppData\Local\Temp\junk.tmp", 'cache')

$hidden = "$src\Documents\hidden.txt"
[IO.File]::WriteAllText($hidden, 'hidden')
[IO.File]::SetAttributes($hidden, [IO.FileAttributes]::Hidden)

$readonly = "$src\Documents\readonly.txt"
[IO.File]::WriteAllText($readonly, 'ro')
[IO.File]::SetAttributes($readonly, [IO.FileAttributes]::ReadOnly)

# "Downloaded from the internet" marker, stored in an alternate data stream.
$motw = "$src\Documents\downloaded.pdf"
[IO.File]::WriteAllText($motw, 'pdf')
Set-Content -LiteralPath $motw -Stream Zone.Identifier -Value "[ZoneTransfer]`r`nZoneId=3"

# A path longer than the classic 260-character limit.
$longRel = 'Documents\' + ('a' * 80) + '\' + ('b' * 80) + '\' + ('c' * 80)
[IO.Directory]::CreateDirectory("$src\$longRel") | Out-Null
[IO.File]::WriteAllText("$src\$longRel\deep.txt", 'deep')

# Legacy junctions, including the self-referencing loop real profiles have.
New-Item -ItemType Junction -Path "$src\Application Data" -Target "$src\AppData\Roaming" | Out-Null
New-Item -ItemType Junction -Path "$src\AppData\Local\Application Data" -Target "$src\AppData\Local" | Out-Null

# A file held open exclusively, like Outlook's mailbox.
$lock = [IO.File]::Open("$src\Documents\mailbox.pst", 'Create', 'ReadWrite', 'None')
$lock.Write([byte[]](1..100), 0, 100)
$lock.Flush()

Write-Host '== Export'
$out = (& $Exe export $src --drive $drive --ticket 1001 --computer SMOKE --no-confirm 2>&1 | Out-String)
$exportExit = $LASTEXITCODE
$lock.Close()
Write-Host (Mask $out)
$code = [regex]::Match($out, '[0-9A-Z]{4}-[0-9A-Z]{4}-[0-9A-Z]{4}').Value

Check ($code -ne '') 'transfer code is shown'
Check ($exportExit -eq 1) 'export exits 1 (finished with issues) because of the locked file'
Check ($out -match 'mailbox\.pst\s+\(in use by another program') 'locked file is reported as in use'
Check ($out -match '2 link\(s\) recorded, not followed') 'junctions are recorded, not followed (no loop)'
Check ($out -match '1 cache/temp location') 'temp folder excluded by rule'

$pkg = (Get-ChildItem (Join-Path $drive 'ProfilePort\transfers') -Directory | Select-Object -First 1).FullName

Write-Host '== List (no code)'
$list = (& $Exe list --drive $drive 2>&1 | Out-String)
Write-Host $list
Check ($list -match '1001\s+.*partial \(1 skipped\)') 'drive listing shows ticket and partial state'

Write-Host '== Verify'
$vout = (& $Exe verify $pkg --code $code 2>&1 | Out-String)
Write-Host (Mask $vout)
Check ($LASTEXITCODE -eq 0) 'verify succeeds'

Write-Host '== Restore to a different username'
$rout = (& $Exe restore $pkg --to $dst --code $code 2>&1 | Out-String)
$restoreExit = $LASTEXITCODE
Write-Host (Mask $rout)

Check ($restoreExit -eq 1) 'restore exits 1 because the backup has a skipped file'
Check ([IO.File]::ReadAllText("$dst\Documents\report.txt") -eq 'quarterly numbers') 'file content restored'
Check ([IO.File]::Exists("$dst\Documents\Ærø – 日本.txt")) 'Unicode file name restored'
Check ([IO.File]::Exists("$dst\$longRel\deep.txt")) 'path longer than 260 characters restored'
Check (-not [IO.File]::Exists("$dst\AppData\Local\Temp\junk.tmp")) 'excluded temp file not restored'
Check (-not (Test-Path -LiteralPath "$dst\Application Data")) 'junction not turned into a copy'
Check (-not [IO.File]::Exists("$dst\Documents\mailbox.pst")) 'locked file not faked'
$leftovers = @(Get-ChildItem $dst -Recurse -Force -File | Where-Object { $_.Name -like '.profileport-*' })
Check ($leftovers.Count -eq 0) 'no temporary files left behind'
Check ([IO.File]::GetLastWriteTimeUtc($readonly) -eq [IO.File]::GetLastWriteTimeUtc("$dst\Documents\readonly.txt")) 'modified time preserved'
Check (([IO.File]::GetAttributes("$dst\Documents\readonly.txt") -band [IO.FileAttributes]::ReadOnly) -ne 0) 'read-only attribute preserved'

Gap (([IO.File]::GetAttributes("$dst\Documents\hidden.txt") -band [IO.FileAttributes]::Hidden) -ne 0) 'hidden attribute preserved'
$stream = Get-Item -LiteralPath "$dst\Documents\downloaded.pdf" -Stream Zone.Identifier -ErrorAction SilentlyContinue
Gap ($null -ne $stream) 'Mark of the Web (alternate data stream) preserved'
Gap ([IO.File]::GetCreationTimeUtc("$src\Documents\report.txt") -eq [IO.File]::GetCreationTimeUtc("$dst\Documents\report.txt")) 'creation time preserved'

Write-Host '== Restore again (as after an interrupted restore)'
$again = (& $Exe restore $pkg --to $dst --code $code 2>&1 | Out-String)
Write-Host (Mask $again)
Check ($again -match 'already there with identical content') 'second restore recognises files already in place'
Check ($again -notmatch 'Name conflicts') 'second restore reports no conflicts'

# Summary
$fail = @($results | Where-Object Result -eq 'FAIL')
$gaps = @($results | Where-Object Result -eq 'GAP')
if ($env:GITHUB_STEP_SUMMARY) {
    $md = "## Windows smoke test`n`n| Result | Check |`n|---|---|`n"
    foreach ($r in $results) { $md += "| $($r.Result) | $($r.Check) |`n" }
    Add-Content -Path $env:GITHUB_STEP_SUMMARY -Value $md
}
Write-Host ''
Write-Host ("{0} passed, {1} known gaps, {2} failed" -f @($results | Where-Object Result -eq 'PASS').Count, $gaps.Count, $fail.Count)
if ($fail.Count -gt 0) { exit 1 }
exit 0
