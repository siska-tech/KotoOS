param(
    [string]$Picotool = $env:PICOTOOL,
    [switch]$AllBins,
    [switch]$ValidationBundle,
    [switch]$WifiResidencyProbe,
    [switch]$WifiSequentialPioProbe,
    [switch]$WifiMinimalProbe,
    [switch]$WifiMinimalDma23Probe,
    [switch]$WifiMinimalCooperativeProbe,
    [switch]$WifiConfig,
    [switch]$NetworkServiceProbe,
    [switch]$AppFetchHttps
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$targetTriple = "thumbv8m.main-none-eabihf"
$features = "board-picocalc-pico2w,ram_interpreter,ram_audio_mixer"
if ($WifiMinimalCooperativeProbe) {
    $features = "$features,wifi_minimal_cooperative_probe"
} elseif ($WifiMinimalDma23Probe) {
    $features = "$features,wifi_minimal_dma23_probe"
} elseif ($WifiSequentialPioProbe) {
    $features = "$features,wifi_pio_sequential_probe"
} elseif ($WifiResidencyProbe) {
    $features = "$features,wifi_residency_probe"
} elseif ($AppFetchHttps) {
    $features = "$features,network_service,app_fetch_https"
} elseif ($WifiConfig -or $NetworkServiceProbe) {
    $features = "$features,network_service"
}
$artifactDir = Join-Path $root "target\$targetTriple\release"
$binName = if ($WifiMinimalProbe -or $WifiMinimalDma23Probe -or $WifiMinimalCooperativeProbe) { "probe_wifi_minimal" } else { "koto_firmware" }
$elf = Join-Path $artifactDir $binName
$uf2Name = if ($WifiMinimalCooperativeProbe) {
    "probe_wifi_minimal-cooperative-csfix-picocalc-pico2w-rp2350a.uf2"
} elseif ($WifiMinimalDma23Probe) {
    "probe_wifi_minimal-dma23-picocalc-pico2w-rp2350a.uf2"
} elseif ($WifiMinimalProbe) {
    "probe_wifi_minimal-picocalc-pico2w-rp2350a.uf2"
} elseif ($WifiSequentialPioProbe) {
    "koto_firmware-picocalc-pico2w-rp2350a-wifi-sequential-pio-probe.uf2"
} elseif ($WifiResidencyProbe) {
    "koto_firmware-picocalc-pico2w-rp2350a-wifi-residency-probe.uf2"
} elseif ($WifiConfig) {
    "koto_firmware-picocalc-pico2w-rp2350a-wifi-config.uf2"
} elseif ($NetworkServiceProbe) {
    "koto_firmware-picocalc-pico2w-rp2350a-network-service.uf2"
} elseif ($AppFetchHttps) {
    "koto_firmware-picocalc-pico2w-rp2350a-app-fetch-https.uf2"
} else {
    "koto_firmware-picocalc-pico2w-rp2350a.uf2"
}
$uf2 = Join-Path $artifactDir $uf2Name
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
    if ($NetworkServiceProbe) {
        Write-Warning "-NetworkServiceProbe is retained for compatibility; use -WifiConfig for the KotoShell product path."
    }
    $binArg = if ($AllBins -or $ValidationBundle) { "--bins" } else { "--bin=$binName" }
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
