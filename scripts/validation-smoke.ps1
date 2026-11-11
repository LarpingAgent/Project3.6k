$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$vsDevCmd = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat'
$vulkanSdk = 'C:\VulkanSDK\1.4.363.0'
$logDirectory = Join-Path $projectRoot 'target\validation-runs'

if (-not (Test-Path -LiteralPath $vsDevCmd -PathType Leaf)) {
    throw "Could not find VsDevCmd.bat at $vsDevCmd"
}
if (-not (Test-Path -LiteralPath (Join-Path $vulkanSdk 'Bin\glslc.exe') -PathType Leaf)) {
    throw "Could not find the Vulkan SDK at $vulkanSdk"
}
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null

$runs = @(
    @{ Name = 'frames-1'; Arguments = '--run-frames 1 --validation' },
    @{ Name = 'frames-60'; Arguments = '--run-frames 60 --validation' },
    @{ Name = 'frames-600'; Arguments = '--run-frames 600 --validation' },
    @{ Name = 'resize-stress'; Arguments = '--run-frames 180 --validation --resize-stress' }
)

Push-Location $projectRoot
try {
    foreach ($run in $runs) {
        $logPath = Join-Path $logDirectory "$($run.Name).log"
        $stdoutPath = Join-Path $logDirectory "$($run.Name).stdout.tmp"
        $stderrPath = Join-Path $logDirectory "$($run.Name).stderr.tmp"
        $command = 'call "{0}" -arch=x64 -host_arch=x64 && set "VULKAN_SDK={1}" && set "RUST_LOG=info" && cargo run -p project3-6k-app -- {2}' -f $vsDevCmd, $vulkanSdk, $run.Arguments
        Write-Host "Running $($run.Name)..."
        $process = Start-Process -FilePath $env:ComSpec -ArgumentList @('/d', '/s', '/c', $command) `
            -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
        $exitCode = $process.ExitCode
        $outputText = @(
            if (Test-Path -LiteralPath $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw }
            if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw }
        ) -join [Environment]::NewLine
        Set-Content -LiteralPath $logPath -Value $outputText
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -ErrorAction SilentlyContinue

        if ($exitCode -ne 0) {
            throw "$($run.Name) exited with code $exitCode. See $logPath"
        }
        if ($outputText -match '(?im)\b(?:WARN|ERROR)\s+vulkan::validation\b|\bVUID-[A-Za-z0-9_-]+') {
            throw "$($run.Name) produced a Vulkan validation warning or error. See $logPath"
        }
        if ($outputText -notmatch 'Vulkan validation clean') {
            throw "$($run.Name) did not report a clean validation summary. See $logPath"
        }
        if ($run.Name -eq 'resize-stress' -and $outputText -notmatch 'zero_extent_events\s*=\s*[1-9][0-9]*') {
            throw "Resize stress did not observe a zero-extent event. See $logPath"
        }
        Write-Host "$($run.Name): passed"
    }
}
finally {
    Pop-Location
}

Write-Host "All validation smoke runs passed. Logs: $logDirectory"
