//! PIO program builders and loaders for the `embassy-rp` RP2040 QPI backend.
//!
//! These helpers were split out of [`super::embassy_hal`] to keep that module
//! smaller. The `pio_asm!` skeletons and timing are reproduced verbatim.

use embassy_rp::pio::{program::pio_asm, Instance, LoadedProgram, PioPin};

use super::{EmbassyRpQpiBackend, EmbassyRpQpiError, TransactionPioFastReadLoopVariant};

impl<'d, PIO, const SM: usize, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
    EmbassyRpQpiBackend<'d, PIO, SM, Sio0, Sio1, Sio2, Sio3, Cs, Sck>
where
    PIO: Instance + 'd,
    Sio0: PioPin + 'd,
    Sio1: PioPin + 'd,
    Sio2: PioPin + 'd,
    Sio3: PioPin + 'd,
    Cs: PioPin + 'd,
    Sck: PioPin + 'd,
{
    pub(super) fn load_programs(&mut self) -> Result<(), EmbassyRpQpiError> {
        if self.spi_command_program.is_none() {
            let program = pio_asm!(
                r#"
                        .side_set 1

                        .wrap_target
                        out pins, 1 side 0 [1]
                        in pins, 1  side 1 [1]
                        .wrap
                    "#
            );
            self.spi_command_program = Some(
                self.common
                    .try_load_program(&program.program)
                    .map_err(|_| EmbassyRpQpiError::ProgramLoad)?,
            );
        }

        if self.qpi_command_program.is_none() {
            let program = pio_asm!(
                r#"
                        .side_set 1

                        .wrap_target
                        out pins, 4 side 0 [1]
                        in pins, 4  side 1 [1]
                        .wrap
                    "#
            );
            self.qpi_command_program = Some(
                self.common
                    .try_load_program(&program.program)
                    .map_err(|_| EmbassyRpQpiError::ProgramLoad)?,
            );
        }

        if self.qpi_write_stream_program.is_none() {
            let program = pio_asm!(
                r#"
                        .side_set 1

                        .wrap_target
                        pull block side 0
                        out x, 32 side 0
                        out pins, 4 side 0 [1]
                        jmp x-- 2 side 1 [1]
                        .wrap
                    "#
            );
            self.qpi_write_stream_program = Some(
                self.common
                    .try_load_program(&program.program)
                    .map_err(|_| EmbassyRpQpiError::ProgramLoad)?,
            );
        }

        if self.qpi_read_stream_program.is_none() {
            let program = pio_asm!(
                r#"
                        .side_set 1

                        .wrap_target
                        pull block side 0
                        out x, 32 side 0
                        nop side 0 [1]
                        in pins, 4 side 1 [1]
                        jmp x-- 2 side 1
                        push block side 0
                        .wrap
                    "#
            );
            self.qpi_read_stream_program = Some(
                self.common
                    .try_load_program(&program.program)
                    .map_err(|_| EmbassyRpQpiError::ProgramLoad)?,
            );
        }

        if self.qpi_transaction_program.is_none() {
            self.ensure_qpi_transaction_program_loaded()?;
        }

        Ok(())
    }

    fn free_loaded_program(&mut self, program: LoadedProgram<'d, PIO>) {
        unsafe {
            self.common.free_instr(program.used_memory);
        }
    }

    pub(super) fn ensure_qpi_transaction_program_loaded(
        &mut self,
    ) -> Result<(), EmbassyRpQpiError> {
        if self.qpi_transaction_program.is_none() {
            if let Some(program) = self.qpi_transaction_fast_program.take() {
                self.free_loaded_program(program);
            }
            self.qpi_transaction_fast_program_variant = None;

            let program = pio_asm!(
                r#"
                        .side_set 2

                        .wrap_target
                        pull block side 0b01
                        out x, 32 side 0b01
                        pull block side 0b01
                        out y, 32 side 0b01
                    txpio_out:
                        out pins, 4 side 0b00 [1]
                        jmp x-- txpio_out side 0b10 [1]
                        set pindirs, 0 side 0b10 [1]
                        nop side 0b00 [1]
                    txpio_in:
                        in pins, 4 side 0b10 [1]
                        jmp y-- txpio_in side 0b00 [1]
                        set pindirs, 15 side 0b01
                        .wrap
                    "#
            );
            self.qpi_transaction_program = Some(
                self.common
                    .try_load_program(&program.program)
                    .map_err(|_| EmbassyRpQpiError::ProgramLoad)?,
            );
        }

        Ok(())
    }

    pub(super) fn ensure_qpi_transaction_fast_program_loaded(
        &mut self,
    ) -> Result<(), EmbassyRpQpiError> {
        let variant = self.transaction_pio_fast_read_loop_variant;
        if self.qpi_transaction_fast_program_variant != Some(variant) {
            if let Some(program) = self.qpi_transaction_fast_program.take() {
                self.free_loaded_program(program);
            }
            self.qpi_transaction_fast_program_variant = None;
        }

        if self.qpi_transaction_fast_program.is_none() {
            if let Some(program) = self.qpi_transaction_program.take() {
                self.free_loaded_program(program);
            }

            self.qpi_transaction_fast_program = Some(match variant {
                TransactionPioFastReadLoopVariant::CurrentNoDelay => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10 [1]
                            nop side 0b00 [1]
                        txpio_in:
                            in pins, 4 side 0b10
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::OppositePolarityNoDelay => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10 [1]
                            nop side 0b00 [1]
                        txpio_in:
                            in pins, 4 side 0b00
                            jmp y-- txpio_in side 0b10
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::DelayOnIn => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10 [1]
                            nop side 0b00 [1]
                        txpio_in:
                            in pins, 4 side 0b10 [1]
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::DelayOnJmp => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10 [1]
                            nop side 0b00 [1]
                        txpio_in:
                            in pins, 4 side 0b10
                            jmp y-- txpio_in side 0b00 [1]
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingFudgeA => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            nop side 0b00
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingFudgeB => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            nop side 0b00
                            jmp txpio_in_mid side 0b00
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingNoFudge => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingFudgeExtraLow => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            nop side 0b00
                            nop side 0b00
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingDiscardFirstNibble => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            in pins, 4 side 0b10
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingExtraDummyHalfCycle => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            nop side 0b00
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
                TransactionPioFastReadLoopVariant::FallingExtraDummyByte => {
                    let program = pio_asm!(
                        r#"
                            .side_set 2

                            .wrap_target
                            pull block side 0b01
                            out x, 32 side 0b01
                            pull block side 0b01
                            out y, 32 side 0b01
                        txpio_out:
                            out pins, 4 side 0b00 [1]
                            jmp x-- txpio_out side 0b10 [1]
                            set pindirs, 0 side 0b10
                            nop side 0b10
                            nop side 0b00
                            nop side 0b10
                            nop side 0b00
                        txpio_in:
                            in pins, 4 side 0b10
                        txpio_in_mid:
                            jmp y-- txpio_in side 0b00
                            set pindirs, 15 side 0b01
                            .wrap
                        "#
                    );
                    self.common
                        .try_load_program(&program.program)
                        .map_err(|_| EmbassyRpQpiError::ProgramLoad)?
                }
            });
            self.qpi_transaction_fast_program_variant = Some(variant);
        }

        Ok(())
    }
}
