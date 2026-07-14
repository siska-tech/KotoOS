MEMORY
{
  /* Raspberry Pi Pico 2 / Pico 2 W: 4 MiB external flash. The RP2350 image
     definition is inserted after the vector table by link-rp235x.x. */
  FLASH : ORIGIN = 0x10000000, LENGTH = 4096K
  /* 512 KiB striped SRAM plus two contiguous 4 KiB scratch banks. */
  RAM   : ORIGIN = 0x20000000, LENGTH = 520K
}
