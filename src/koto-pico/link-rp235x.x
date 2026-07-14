/* RP235x Boot ROM image definition.
 *
 * cortex-m-rt places .text immediately after .vector_table by default. Insert
 * embassy-rp's IMAGE_DEF block between them so the Boot ROM can find it in the
 * first 4 KiB of flash, then move .text past the inserted block.
 */
SECTIONS
{
  .start_block : ALIGN(4)
  {
    __start_block_addr = .;
    KEEP(*(.start_block));
    KEEP(*(.boot_info));
  } > FLASH
}
INSERT AFTER .vector_table;

_stext = ADDR(.start_block) + SIZEOF(.start_block);
