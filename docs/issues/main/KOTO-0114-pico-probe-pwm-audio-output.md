# KOTO-0114: Pico Probe: PWM Audio Output

- Status: done
- Type: feature
- Priority: P1
- Requirements: HC-4, FR-MML-2, NFR-REL-3

## Goal

Prove the PicoCalc hardware audio path by driving GP26/GP27 through their
shared PWM slice from a bounded PCM ring buffer without disrupting the device
loop.

## Acceptance Criteria

- [x] The probe configures an out-of-audible-range PWM carrier between 64 and
  128 kHz and logs the achieved frequency.
- [x] A deterministic PCM test tone is audible through the PicoCalc speaker or
  headphone output.
- [x] Ring-buffer and DMA/timer feeding runs without underruns during the
  observation interval.
- [x] Bring-up notes record output path, sample rate, carrier rate, buffer
  size, and observed sound quality.

## Notes

This is probe 6 from `docs/RP2040_BRINGUP.md`.

It builds on the portable mixer from KOTO-0023. GP26 and GP27 share PWM slice
5, so this probe must preserve the software-mixed PCM model rather than treating
the two pins as independent hardware tone generators. Use UART0/GP0 at 115200
8N1 for diagnostics so the PicoCalc Type-C connection can power the board and
carry logs.

Implementation started on 2026-06-20. The `pwm_audio` probe drives PWM slice 5
on GP26/GP27 with a 6:1 divider and TOP=250. With the default 125 MHz system
clock this yields an approximately 83.0 kHz carrier. A 440 Hz deterministic
triangle wave was used by the initial timer-fed implementation. The corrected
DMA implementation renders a deterministic 500 Hz triangle wave through the
KOTO-0023 `PcmMixer` into a bounded 256-sample, 1024-byte-aligned DMA ring.
The tone has exactly 16 samples per cycle at 8 kHz and therefore wraps
continuously at the DMA ring boundary.

The probe emits a three-second tone followed by two seconds of silence. UART
diagnostics are written only during silence.

The first physical run on 2026-06-21 exposed a feeder design error: the Embassy
async timer woke 181-183 microseconds after each requested deadline, longer
than the 125-microsecond sample period. All 24,000 samples were consequently
reported as underruns. Async task wake latency is not a suitable PCM sample
clock.

The feeder now uses RP2040 DMA channel 0, paced by DMA timer 0 at exactly 8 kHz.
DMA writes packed channel-A/channel-B compare values directly into PWM slice
5's compare register and wraps its read address over the aligned 256-sample
ring. The CPU and Embassy timer are no longer involved in per-sample delivery.
The corrective physical run on 2026-06-21 repeatedly reported
`dma_remaining=0`, `dma_error=0`, `underruns=0`, and `result=pass` for each
24,000-sample interval. The 500 Hz tone was clearly audible through the
PicoCalc speaker.

## Resolution

PWM slice 5 now produces an approximately 83.0 kHz carrier on GP26/GP27 while
DMA timer 0 provides deterministic 8 kHz PCM pacing. The bounded ring completed
repeated three-second intervals without underruns or DMA errors, and physical
speaker output was confirmed.
