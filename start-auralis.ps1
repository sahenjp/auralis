$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$modelDirectory = Join-Path $root 'models'
$modelPath = Join-Path $modelDirectory 'ulunas_stream_simple.onnx'
$modelUrl = 'https://huggingface.co/j-llm/Auralis/resolve/d6fe7e57b4f3c3d2744bcd74bf0cbe37b9e1aa55/ulunas_stream_simple.onnx'
$expectedBytes = 788967L
$expectedSha256 = 'f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b'

if (-not (Test-Path -LiteralPath $modelPath -PathType Leaf)) {
    New-Item -ItemType Directory -Force -Path $modelDirectory | Out-Null
    $temporaryPath = Join-Path $modelDirectory ('.ulunas-' + [guid]::NewGuid().ToString('N') + '.download')
    try {
        Write-Host 'Downloading the pinned UL-UNAS model...'
        & curl.exe --fail --location --silent --show-error --max-filesize $expectedBytes --output $temporaryPath $modelUrl
        if ($LASTEXITCODE -ne 0) {
            throw 'Model download failed.'
        }

        $downloaded = Get-Item -LiteralPath $temporaryPath
        if ($downloaded.Length -ne $expectedBytes) {
            throw "Unexpected model size: $($downloaded.Length) bytes."
        }
        $actualSha256 = (Get-FileHash -LiteralPath $temporaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actualSha256 -ne $expectedSha256) {
            throw "Model SHA-256 mismatch: $actualSha256"
        }
        [System.IO.File]::Move($temporaryPath, $modelPath)
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
    }
}

$localModel = Get-Item -LiteralPath $modelPath
if ($localModel.Length -ne $expectedBytes) {
    throw "Unexpected model size: $($localModel.Length) bytes."
}
$localSha256 = (Get-FileHash -LiteralPath $modelPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($localSha256 -ne $expectedSha256) {
    throw "Model SHA-256 mismatch: $localSha256"
}

Set-Location $root
$guiArgs = @('gui', '--profile', 'balanced', '--model', $modelPath)
$executable = Join-Path $root 'auralis-cli.exe'
if (Test-Path -LiteralPath $executable -PathType Leaf) {
    & $executable @guiArgs
}
else {
    $cargoArgs = @('run', '--release', '-p', 'auralis-cli', '--') + $guiArgs
    & cargo @cargoArgs
}
exit $LASTEXITCODE
