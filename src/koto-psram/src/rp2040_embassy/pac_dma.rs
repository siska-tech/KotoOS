use core::sync::atomic::{compiler_fence, Ordering};

use embassy_rp::pac::{
    self,
    dma::vals::{DataSize, TreqSel},
};

use super::transaction_pio::PacDmaStatus;
use super::EmbassyRpQpiError;

pub(super) const TX_DMA_PAC_CH: usize = 0;
pub(super) const TX_DMA_PAC_CH_MASK: u16 = 1 << TX_DMA_PAC_CH;
pub(super) const RX_DMA_PAC_CH: usize = 1;
pub(super) const RX_DMA_PAC_CH_MASK: u16 = 1 << RX_DMA_PAC_CH;

pub(super) fn configure_tx_dma_ch0(
    read_addr: u32,
    write_addr: u32,
    trans_count: usize,
    ctrl_base: u32,
) -> (u32, PacDmaStatus) {
    let ch = pac::DMA.ch(TX_DMA_PAC_CH);
    let before_arm = tx_dma_ch0_status();

    ch.write_addr().write_value(write_addr);
    ch.read_addr().write_value(read_addr);
    #[cfg(feature = "rp2040-embassy")]
    ch.trans_count().write_value(trans_count as u32);
    #[cfg(feature = "rp235xa-embassy")]
    ch.trans_count().write(|w| w.set_count(trans_count as u32));

    compiler_fence(Ordering::SeqCst);
    ch.ctrl_trig()
        .write_value(pac::dma::regs::CtrlTrig(ctrl_base | 1));
    compiler_fence(Ordering::SeqCst);

    (before_arm.ctrl_trig, tx_dma_ch0_status())
}

pub(super) fn tx_dma_ch0_ctrl_base(treq: TreqSel) -> u32 {
    let mut ctrl = pac::dma::regs::CtrlTrig::default();
    ctrl.set_treq_sel(treq);
    ctrl.set_data_size(DataSize::SIZE_WORD);
    ctrl.set_incr_read(true);
    ctrl.set_incr_write(false);
    ctrl.set_chain_to(TX_DMA_PAC_CH as u8);
    ctrl.set_irq_quiet(true);
    ctrl.set_bswap(false);
    ctrl.set_sniff_en(false);
    ctrl.set_en(false);
    ctrl.0
}

pub(super) fn configure_rx_dma_ch1(
    read_addr: u32,
    write_addr: u32,
    trans_count: usize,
    ctrl_base: u32,
) -> (u32, PacDmaStatus) {
    let ch = pac::DMA.ch(RX_DMA_PAC_CH);
    let before_arm = rx_dma_ch1_status();

    ch.write_addr().write_value(write_addr);
    ch.read_addr().write_value(read_addr);
    #[cfg(feature = "rp2040-embassy")]
    ch.trans_count().write_value(trans_count as u32);
    #[cfg(feature = "rp235xa-embassy")]
    ch.trans_count().write(|w| w.set_count(trans_count as u32));

    compiler_fence(Ordering::SeqCst);
    ch.ctrl_trig()
        .write_value(pac::dma::regs::CtrlTrig(ctrl_base | 1));
    compiler_fence(Ordering::SeqCst);

    (before_arm.ctrl_trig, rx_dma_ch1_status())
}

pub(super) fn rx_dma_ch1_ctrl_base(treq: TreqSel) -> u32 {
    let mut ctrl = pac::dma::regs::CtrlTrig::default();
    ctrl.set_treq_sel(treq);
    ctrl.set_data_size(DataSize::SIZE_WORD);
    ctrl.set_incr_read(false);
    ctrl.set_incr_write(true);
    ctrl.set_chain_to(RX_DMA_PAC_CH as u8);
    ctrl.set_irq_quiet(true);
    ctrl.set_bswap(false);
    ctrl.set_sniff_en(false);
    ctrl.set_en(false);
    ctrl.0
}

pub(super) fn rx_dma_ch1_u8_ctrl_base(treq: TreqSel) -> u32 {
    let mut ctrl = pac::dma::regs::CtrlTrig::default();
    ctrl.set_treq_sel(treq);
    ctrl.set_data_size(DataSize::SIZE_BYTE);
    ctrl.set_incr_read(false);
    ctrl.set_incr_write(true);
    ctrl.set_chain_to(RX_DMA_PAC_CH as u8);
    ctrl.set_irq_quiet(true);
    ctrl.set_bswap(false);
    ctrl.set_sniff_en(false);
    ctrl.set_en(false);
    ctrl.0
}

pub(super) fn prepare_tx_dma_ch0(
    timeout_polls: u32,
) -> Result<(PacDmaStatus, PacDmaStatus), EmbassyRpQpiError> {
    let ch = pac::DMA.ch(TX_DMA_PAC_CH);
    let before = tx_dma_ch0_status();

    if before.busy && wait_tx_dma_ch0_idle(timeout_polls).is_err() {
        abort_tx_dma_ch0(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }

    ch.ctrl_trig().modify(|w| {
        w.set_read_error(true);
        w.set_write_error(true);
    });
    clear_tx_dma_ch0_irq();

    let after = tx_dma_ch0_status();
    if after.busy {
        abort_tx_dma_ch0(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }
    if after.read_error || after.write_error || after.ahb_error {
        abort_tx_dma_ch0(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }

    Ok((before, after))
}

pub(super) fn prepare_rx_dma_ch1(
    timeout_polls: u32,
) -> Result<(PacDmaStatus, PacDmaStatus), EmbassyRpQpiError> {
    let ch = pac::DMA.ch(RX_DMA_PAC_CH);
    let before = rx_dma_ch1_status();

    if before.busy && wait_rx_dma_ch1_idle(timeout_polls).is_err() {
        abort_rx_dma_ch1(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }

    ch.ctrl_trig().modify(|w| {
        w.set_read_error(true);
        w.set_write_error(true);
    });
    clear_rx_dma_ch1_irq();

    let after = rx_dma_ch1_status();
    if after.busy {
        abort_rx_dma_ch1(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }
    if after.read_error || after.write_error || after.ahb_error {
        abort_rx_dma_ch1(timeout_polls);
        return Err(EmbassyRpQpiError::Timeout);
    }

    Ok((before, after))
}

pub(super) fn poll_tx_dma_ch0_until_done(timeout_polls: u32) -> Result<(), EmbassyRpQpiError> {
    for _ in 0..timeout_polls {
        let status = tx_dma_ch0_status();
        if status.read_error || status.write_error || status.ahb_error {
            return Err(EmbassyRpQpiError::Timeout);
        }
        if !status.busy {
            return Ok(());
        }
    }

    Err(EmbassyRpQpiError::Timeout)
}

pub(super) fn poll_rx_dma_ch1_until_done(timeout_polls: u32) -> Result<(), EmbassyRpQpiError> {
    for _ in 0..timeout_polls {
        let status = rx_dma_ch1_status();
        if status.read_error || status.write_error || status.ahb_error {
            return Err(EmbassyRpQpiError::Timeout);
        }
        if !status.busy {
            return Ok(());
        }
    }

    Err(EmbassyRpQpiError::Timeout)
}

fn wait_tx_dma_ch0_idle(timeout_polls: u32) -> Result<(), EmbassyRpQpiError> {
    for _ in 0..timeout_polls {
        if !tx_dma_ch0_status().busy {
            return Ok(());
        }
    }

    Err(EmbassyRpQpiError::Timeout)
}

fn wait_rx_dma_ch1_idle(timeout_polls: u32) -> Result<(), EmbassyRpQpiError> {
    for _ in 0..timeout_polls {
        if !rx_dma_ch1_status().busy {
            return Ok(());
        }
    }

    Err(EmbassyRpQpiError::Timeout)
}

pub(super) fn cleanup_tx_dma_ch0(timeout_polls: u32) -> PacDmaStatus {
    let ch = pac::DMA.ch(TX_DMA_PAC_CH);
    let _ = wait_tx_dma_ch0_idle(timeout_polls);
    ch.ctrl_trig().modify(|w| {
        w.set_en(false);
    });
    clear_tx_dma_ch0_irq();
    tx_dma_ch0_status()
}

pub(super) fn cleanup_rx_dma_ch1(timeout_polls: u32) -> PacDmaStatus {
    let ch = pac::DMA.ch(RX_DMA_PAC_CH);
    let _ = wait_rx_dma_ch1_idle(timeout_polls);
    ch.ctrl_trig().modify(|w| {
        w.set_en(false);
    });
    clear_rx_dma_ch1_irq();
    rx_dma_ch1_status()
}

pub(super) fn disable_rx_dma_ch1_now() -> PacDmaStatus {
    let ch = pac::DMA.ch(RX_DMA_PAC_CH);
    ch.ctrl_trig().modify(|w| {
        w.set_en(false);
    });
    clear_rx_dma_ch1_irq();
    rx_dma_ch1_status()
}

pub(super) fn tx_dma_ch0_status() -> PacDmaStatus {
    let ctrl = pac::DMA.ch(TX_DMA_PAC_CH).ctrl_trig().read();
    PacDmaStatus {
        busy: ctrl.busy(),
        read_error: ctrl.read_error(),
        write_error: ctrl.write_error(),
        ahb_error: ctrl.ahb_error(),
        ctrl_trig: ctrl.0,
    }
}

pub(super) fn rx_dma_ch1_status() -> PacDmaStatus {
    let ctrl = pac::DMA.ch(RX_DMA_PAC_CH).ctrl_trig().read();
    PacDmaStatus {
        busy: ctrl.busy(),
        read_error: ctrl.read_error(),
        write_error: ctrl.write_error(),
        ahb_error: ctrl.ahb_error(),
        ctrl_trig: ctrl.0,
    }
}

fn clear_tx_dma_ch0_irq() {
    pac::DMA.intr(0).write_value(TX_DMA_PAC_CH_MASK as u32);
}

fn clear_rx_dma_ch1_irq() {
    pac::DMA.intr(0).write_value(RX_DMA_PAC_CH_MASK as u32);
}

pub(super) fn abort_tx_dma_ch0(timeout_polls: u32) {
    let ch = pac::DMA.ch(TX_DMA_PAC_CH);
    ch.ctrl_trig().write(|w| {
        w.set_chain_to(TX_DMA_PAC_CH as u8);
        w.set_en(false);
    });
    pac::DMA.chan_abort().write(|w| {
        w.set_chan_abort(TX_DMA_PAC_CH_MASK);
    });
    for _ in 0..timeout_polls {
        if pac::DMA.chan_abort().read().chan_abort() & TX_DMA_PAC_CH_MASK == 0 {
            break;
        }
    }
}

pub(super) fn abort_rx_dma_ch1(timeout_polls: u32) {
    let ch = pac::DMA.ch(RX_DMA_PAC_CH);
    ch.ctrl_trig().write(|w| {
        w.set_chain_to(RX_DMA_PAC_CH as u8);
        w.set_en(false);
    });
    pac::DMA.chan_abort().write(|w| {
        w.set_chan_abort(RX_DMA_PAC_CH_MASK);
    });
    for _ in 0..timeout_polls {
        if pac::DMA.chan_abort().read().chan_abort() & RX_DMA_PAC_CH_MASK == 0 {
            break;
        }
    }
}
