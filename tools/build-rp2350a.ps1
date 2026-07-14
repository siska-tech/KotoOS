param(
    [string]$Picotool = $env:PICOTOOL,
    [switch]$AllBins,
    [switch]$ValidationBundle
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$targetTriple = "thumbv8m.main-none-eabihf"
$features = "board-picocalc-pico2w,ram_interpreter,ram_audio_mixer"
$artifactDir = Join-Path $root "target\$targetTriple\release"
$elf = Join-Path $artifactDir "koto_firmware"
$uf2 = Join-Path $artifactDir "koto_firmware-picocalc-pico2w-rp2350a.uf2"
$probeBins = @(
    "probe_lcd",
    "probe_keyboard",
    "probe_sd",
    "probe_psram",
    "probe_power",
    "probe_audio"
)

if (-not $Picotool) {
    $command = Get-Command picotool -ErrorAction SilentlyContinue
    if ($command) {
        $Picotool = $command.Source
    }
}
if (-not $Picotool -or -not (Test-Path -LiteralPath $Picotool)) {
    throw "picotool was not found. Install Raspberry Pi picotool 2.x or pass -Picotool <path>."
}

Push-Location $root
try {
    $binArg = if ($AllBins -or $ValidationBundle) { "--bins" } else { "--bin=koto_firmware" }
    & cargo build -p koto-pico $binArg --release --target $targetTriple `
        --no-default-features --features $features
    if ($LASTEXITCODE -ne 0) {
        throw "RP2350A Cargo build failed with exit code $LASTEXITCODE"
    }

    # RP2350 Arm secure UF2 family plus the official RP2350-E10 absolute block.
    # elf2uf2-rs emits the RP2040 family ID and must not be used for this image.
    & $Picotool uf2 convert $elf -t elf $uf2 -t uf2 `
        --family 0xe48bff59 --platform rp2350 --abs-block
    if ($LASTEXITCODE -ne 0) {
        throw "picotool UF2 conversion failed with exit code $LASTEXITCODE"
    }

    & $Picotool info -a $uf2 -t uf2
    if ($LASTEXITCODE -ne 0) {
        throw "picotool could not inspect the generated UF2"
    }
    $outputs = @($uf2)

    if ($ValidationBundle) {
        foreach ($probe in $probeBins) {
            $probeElf = Join-Path $artifactDir $probe
            $probeUf2 = Join-Path $artifactDir "$probe-picocalc-pico2w-rp2350a.uf2"
            & $Picotool uf2 convert $probeElf -t elf $probeUf2 -t uf2 `
                --family 0xe48bff59 --platform rp2350 --abs-block
            if ($LASTEXITCODE -ne 0) {
                throw "picotool conversion failed for $probe with exit code $LASTEXITCODE"
            }
            $outputs += $probeUf2
        }

        $fallbackFeatures = "$features,force_psram_fallback"
        & cargo build -p koto-pico --bin=koto_firmware --release --target $targetTriple `
            --no-default-features --features $fallbackFeatures
        if ($LASTEXITCODE -ne 0) {
            throw "RP2350A forced-fallback build failed with exit code $LASTEXITCODE"
        }
        $fallbackUf2 = Join-Path $artifactDir `
            "koto_firmware-picocalc-pico2w-rp2350a-forced-psram-fallback.uf2"
        & $Picotool uf2 convert $elf -t elf $fallbackUf2 -t uf2 `
            --family 0xe48bff59 --platform rp2350 --abs-block
        if ($LASTEXITCODE -ne 0) {
            throw "picotool conversion failed for forced-fallback firmware"
        }
        $outputs += $fallbackUf2
    }

    $outputs | ForEach-Object { Write-Output $_ }
}
finally {
    Pop-Location
}
