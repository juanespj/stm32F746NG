/* STM32F746NG memory map
 *
 * Flash: 1 MB  @ 0x08000000
 * DTCM:  64 KB @ 0x20000000   (fast, tightly-coupled — good for stacks)
 * SRAM1:256 KB @ 0x20010000   \  mapped contiguously as
 * SRAM2: 16 KB @ 0x20050000   /  one 272 KB AXI SRAM region
 *
 * Our RGB565 framebuffer: 480 * 272 * 2 = 261 120 bytes (~255 KB)
 * We map all of DTCM + SRAM1 + SRAM2 as a single 320 KB RAM region so the
 * linker can place the framebuffer wherever it fits.
 */
MEMORY
{
    FLASH : ORIGIN = 0x08000000, LENGTH = 1024K
    RAM   : ORIGIN = 0x20000000, LENGTH = 320K
}
