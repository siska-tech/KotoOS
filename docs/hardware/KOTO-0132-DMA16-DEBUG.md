# KOTO-0132 DMA16 Timeout Debugging Guide

## Current Status (2026-06-25)

- Embassy SM1 CPU-TX/RX-DMA PSRAM read path is working and integrated behind a feature-gated CodeWindow experiment.
- Read sizes 1B through 4096B have been validated in diagnostics.
- Boundary and stress diagnostics are passing.
- KotoBlocks launches successfully with `psram_dma_read_code_window` enabled.
- Observed effective refill bandwidth during app execution is about 1.3-1.4 MB/s.
- TX DMA and QPI remain future optimization phases and are intentionally out of scope for this working-state checkpoint.

## Overview

The KOTO-0132 phase 3g real DMA implementation includes comprehensive diagnostic logging to identify why DMA transfers may timeout during the 16-byte PSRAM read operation.

## DMA16 Diagnostic Fields

### Configuration Fields (Constant)
- **dma_ch**: DMA channel number (always 1 to avoid LCD/audio on CH0)
- **pio_inst**: PIO instance (always 1 for PSRAM backend)
- **sm**: State machine index (always 0 for PSRAM backend)
- **treq**: TREQ/DREQ selection (always PIO1_RX0 for PIO1 SM0 RX FIFO)
- **src**: Source address (PIO1 RX FIFO register address, typically 0x4002xxxx)
- **dst**: Destination address (SRAM buffer address for DMA write)

### Transfer Count Fields
- **tc_before**: Transfer count before DMA ARM (should be 0 or garbage)
- **tc_after_arm**: Transfer count after DMA ARM (should be 16)
- **tc_after_cmd**: Transfer count after PSRAM read command issued (should be 16)
- **tc_timeout**: Transfer count when timeout occurs (indicates how many words were transferred)

**Interpretation:**
- If `tc_after_arm ≠ 16`: DMA register write failed or not properly configured
- If `tc_after_cmd ≠ 16`: Command may have triggered DMA early
- If `tc_timeout = 16`: DMA never transferred any words (DREQ not working)
- If `tc_timeout < 16` and close to 16 (e.g., 15, 14): DMA transferred some words then stalled
- If `tc_timeout ≈ 0`: DMA transferred most/all words but never signaled completion

### Control Register Fields
- **ctrl_before**: Raw ctrl_trig value before ARM (enable bit should be 0)
- **ctrl_after_arm**: Raw ctrl_trig value after ARM (enable bit should be 1, busy should be 0 initially)
- **ctrl_after_cmd**: Raw ctrl_trig value after command issued (busy should become 1 if DMA receives DREQ)
- **ctrl_timeout**: Raw ctrl_trig value when timeout occurs (busy should still be 1)

**Register Bit Layout (simplified):**
- Bit 0: `en` (enable)
- Bit 24: `busy` (transfer in progress)
- Bits 10-15: `treq_sel` (DREQ source selection)
- Bit 31: `ahb_error` (AHB error flag)

**Interpretation:**
- Check bit 0 (en): should be 1 after ARM and during transfer
- Check bit 24 (busy): should be 0→1→0, or 0→1 then timeout if stalled
- If busy never becomes 1: DMA never received DREQ (SM/PIO issue)
- If busy stays 1 at timeout: DMA transfer incomplete (data stalled in RX FIFO)
- If `ahb_error` bit is set: Bus error during transfer

### RX FIFO Level Fields
- **pio_rx_level_before**: RX FIFO level before command (currently placeholder)
- **pio_rx_level_after_cmd**: RX FIFO level after command issued (currently placeholder)
- **pio_rx_level_timeout**: RX FIFO level at timeout (currently placeholder)

**Note:** RP2040 PIO does not expose RX FIFO level directly without side effects (reading level register may consume FIFO data). These fields are placeholders for future enhancement.

### Error Flags
- **ahb_error**: AHB (AMBA High-performance Bus) error detected during transfer
- **timed_out**: Flag indicating this is a timeout error (true = 10ms timeout reached, false = error detected before timeout)

## Expected UART Output Example

### Successful DMA Transfer (No Error)
```
dma16 read_us=500 verify=pass dma_vs_prod=pass dma_vs_pio=pass
```

### DMA Timeout with Diagnostic Info
```
dma16 error=DmaTimeout reason=dma_timeout read_us=10237
  dma_ch=1 pio_inst=1 sm=0 treq=PIO1_RX0
  src=0x4002xxxx dst=0x20000xxx
  tc_before=0 tc_after_arm=16 tc_after_cmd=16 tc_timeout=16
  ctrl_before=0x00000000 ctrl_after_arm=0x00000001
  ctrl_after_cmd=0x00000001 ctrl_timeout=0x00000001
  ahb_err=false busy_never=false
```

## Diagnostic Interpretation Scenarios

### Scenario 1: DMA Never Starts (busy never becomes true)
**Observed:** `tc_timeout=16 ctrl_timeout=0x00000001 busy_never=true`

**Meaning:** DMA channel was armed but received no DREQ signal.

**Likely causes:**
1. PIO1 SM0 not running or TX FIFO stalled
2. TREQ_SEL::PIO1_RX0 mapping incorrect
3. SM output pin (MISO) not properly configured
4. PSRAM command bytes not reaching SM

**Next debug steps:**
- Check if pio_word16 collects correct data (it does work per hardware log)
- Verify SM state and instruction pointer
- Check if SM RX FIFO is populated after command issue
- Measure SM clock divider and bit timing

### Scenario 2: DMA Receives Data But Transfers All Words Before Stalling
**Observed:** `tc_timeout=0 ctrl_timeout=0x00000001`

**Meaning:** DMA transferred all 16 words but never signaled completion.

**Likely causes:**
1. DMA completion interrupt never fires (not configured)
2. DMA busy flag stuck at 1 due to hardware error
3. Bus hang or deadlock

**Next debug steps:**
- Check if DMA transfer actually wrote to destination buffer (compare data)
- Force-clear DMA channel and retry
- Check for bus arbitration issues

### Scenario 3: DMA Transfers Partial Words Then Stalls
**Observed:** `tc_timeout=8 ctrl_timeout=0x00000001`

**Meaning:** DMA transferred 8 of 16 words then stopped receiving DREQ.

**Likely causes:**
1. SM RX FIFO underrun (PIO not producing data fast enough)
2. CPU task preemption or high-priority interrupt interfering
3. PIO program bug or SM state corruption
4. Incorrect `in_bits_count` causing SM to stop early

**Next debug steps:**
- Verify PSRAM timing expectations are met
- Check if SM executes full 127-bit loop for 16B read
- Look for CPU interrupt or context switch during DMA
- Compare with pio_word16 transfer rate

### Scenario 4: AHB Error During Transfer
**Observed:** `ahb_err=true tc_timeout=4 (any value)`

**Meaning:** DMA bus error occurred.

**Likely causes:**
1. Destination address misaligned or outside valid SRAM
2. Source address invalid
3. DMA configuration conflict with other bus master
4. Peripheral bus parity/ECC error

**Next debug steps:**
- Verify SRAM buffer address is word-aligned
- Check PIO RX FIFO address calculation
- Disable other DMA channels or probes temporarily
- Check for memory protection violations

## Production Code Verification

The implementation ensures:
- ✅ `read_increment = false`: PIO RX FIFO address not incremented (fixed read)
- ✅ `write_increment = true`: SRAM destination address incremented per word
- ✅ `data_size = WORD`: 32-bit word transfers (matches PIO FIFO width)
- ✅ `transfer_count = 16`: 16 words (64 bytes data, 16 bytes payload after byte extraction)
- ✅ DMA armed before PSRAM command issued (ensures ready state)
- ✅ RX FIFO clear before ARM (no stale data)
- ✅ PSRAM command sequence identical to pio_word16

## Debugging Checklist

1. **Check Configuration**
   - Verify dma_ch=1, pio_inst=1, sm=0
   - Confirm treq=PIO1_RX0 mapped correctly
   - src address matches PIO1.rxf(0) register
   - dst address is word-aligned SRAM buffer

2. **Trace Transfer Progress**
   - tc_after_arm should be 16
   - Check if tc_timeout decreases (DMA starts)
   - If tc_timeout stays 16, DMA never received DREQ
   - If tc_timeout reaches 0, DMA completed but didn't signal

3. **Check Control Register Evolution**
   - ctrl_before should have en=0
   - ctrl_after_arm should have en=1, busy=0
   - ctrl_after_cmd should have busy=0→1 if DREQ arrives
   - ctrl_timeout should reflect final state

4. **Verify RX FIFO Health**
   - pio_word16 works → pio_word_buf is populated
   - If DMA tc_timeout=16 → RX FIFO data not flowing to DMA
   - If DMA tc_timeout<16 → RX FIFO data partial or stuck

5. **Compare Reference Paths**
   - prod16: reference (always works)
   - pio_word16: CPU-pumped (diagnostic reference)
   - dma16: real DMA (under debug)

## Expected UART Output for Working Scenario

```
prod16 verify=pass
pio_word16 verify=pass pio_vs_prod=pass
dma16 read_us=480 verify=pass dma_vs_prod=pass dma_vs_pio=pass
```

## Expected UART Output for Timeout Scenario (Before Fix)

```
prod16 verify=pass
pio_word16 verify=pass pio_vs_prod=pass
dma16 error=DmaTimeout reason=dma_timeout read_us=1221
  dma_ch=1 pio_inst=1 sm=0 treq=PIO1_RX0
  src=0x4002xxxx dst=0x20000yyy
  tc_before=0 tc_after_arm=16 tc_after_cmd=16 tc_timeout=16
  ctrl_before=0x00000000 ctrl_after_arm=0x00000001
  ctrl_after_cmd=0x00000001 ctrl_timeout=0x00000001
  ahb_err=false busy_never=false
```

This indicates DMA was armed correctly but never received DREQ signal from SM0 RX FIFO.

## No Production Impact

All diagnostic logging is feature-gated behind `psram_pio_word_diag` compile-time feature and does not affect:
- Production PsramHal::read() path
- PsramBlocks or PsramCodeWindow
- CODE_WINDOW_BYTES or app launch
- DMA CH0 (LCD scanline, PWM audio)
- PIO program or SM behavior
