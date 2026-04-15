# STM32F746G-DISCO Display — Embassy Example

A minimal working example for driving the on-board 480×272 LCD on the
**STM32F746G Discovery** kit using the
[Embassy](https://embassy.dev/) async embedded framework.

## What it does

1. Configures the STM32F746NG system clock to **216 MHz** (HSE 25 MHz → PLLSAI).
2. Enables the LCD panel (DISP pin) and backlight (BL_CTRL pin).
3. Initialises Embassy's built-in **LTDC driver** with the correct timing
   parameters for the on-board Rocktech **RK043FN48H** 4.3" display.
4. Creates a single full-screen **RGB565** layer.
5. Runs an animation loop that renders a scrolling colour gradient into a static
   frame buffer and asynchronously hands each frame to the display.

---

## Prerequisites

### Toolchain

```bash
# Install the Rust nightly toolchain (Embassy requires it)
rustup toolchain install nightly
rustup default nightly
rustup target add thumbv7em-none-eabihf
```

### Flashing / debugging tools

```bash
cargo install probe-rs-tools   # gives you `probe-rs` for flashing
cargo install flip-link        # stack-overflow protection linker
```

Connect the board to your PC via the **USB ST-LINK** connector (CN14, the
micro-USB port next to the ARDUINO connectors).

---

## Build & flash

```bash
# Debug build and flash
cargo run

# Release build and flash (fits in 1 MB flash, runs much faster)
cargo run --release
```

`probe-rs` will detect the ST-LINK automatically and flash the binary.

### RTT logging

`defmt` logs are printed via RTT. You can view them with:

```bash
probe-rs attach --chip STM32F746NGHx   # then open an RTT terminal
```

or simply with:

```bash
cargo run 2>&1 | defmt-print -e target/thumbv7em-none-eabihf/debug/stm32f746-disco-display
```

---

## Key files

| File | Purpose |
|------|---------|
| `src/main.rs` | All application code — clock setup, GPIO, LTDC init, render loop |
| `memory.x` | Linker script — Flash & RAM regions for STM32F746NG |
| `Cargo.toml` | Dependencies (embassy-stm32, embassy-executor, embassy-time) |
| `.cargo/config.toml` | Target triple, `probe-rs` runner, linker flags |

---

## Hardware pin mapping

| Signal | MCU Pin | Board connector |
|--------|---------|-----------------|
| LCD_CLK | PI14 | — |
| LCD_HSYNC | PI10 | — |
| LCD_VSYNC | PI9 | — |
| LCD_DE | PK7 | — |
| LCD_R[7:3] | PI15, PJ0, PJ1, PJ2, PJ3 | — |
| LCD_G[7:2] | PJ7, PJ8, PJ9, PJ10, PK0, PK1 | — |
| LCD_B[7:3] | PK2, PJ13, PJ14, PJ15, PE4 | — |
| LCD_DISP | PI12 | — |
| LCD_BL_CTRL | PK3 | — |

Source: ST UM1907 Rev 6, Table 14.

---

## Adapting this example

### Drawing with `embedded-graphics`

Add `embedded-graphics` to `Cargo.toml`:

```toml
embedded-graphics = "0.8"
```

Then wrap the framebuffer in a simple `DrawTarget`:

```rust
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};

struct Fb<'a>(&'a mut [u16; FB_SIZE]);

impl DrawTarget for Fb<'_> {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(point, color) in pixels {
            if point.x >= 0 && point.x < LCD_WIDTH as i32
                && point.y >= 0 && point.y < LCD_HEIGHT as i32
            {
                let idx = point.y as usize * LCD_WIDTH as usize + point.x as usize;
                self.0[idx] = color.into_storage();
            }
        }
        Ok(())
    }

    fn bounding_box(&self) -> Rectangle {
        Rectangle::new(Point::zero(), Size::new(LCD_WIDTH as u32, LCD_HEIGHT as u32))
    }
}
```

### Double buffering

For tear-free rendering allocate two static frame buffers and ping-pong between
them while the display scans out the previous frame.

---

## Troubleshooting

| Symptom | Likely cause |
|---------|-------------|
| Screen stays black | Check LCD_DISP (PI12) is high; check PLLSAI clock |
| Distorted image | Wrong pixel clock — adjust `pllsai_divr` |
| Compile error on LTDC pins | Ensure embassy-stm32 git version (crates.io may lag) |
| `probe-rs` cannot find device | Use the ST-LINK USB port, not the OTG port |