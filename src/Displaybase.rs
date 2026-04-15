//! Embassy LTDC Display Example for STM32F746G-DISCO
//!
//! Targets embassy-stm32 0.6.0 / embassy-executor 0.10.0 / embassy-time 0.5.1
//!
//! new_with_pins signature (29 args, confirmed from 0.6.0 source):
//!   (peri, irqs, clk, hsync, vsync, b0..b7, g0..g7, r0..r7)
//!
//! All pins verified against embassy 0.6.0 trait impls for stm32f746ng:
//!   CLK   = PI14   HSYNC = PI10   VSYNC = PI9
//!   B0=PE4   B1=PJ13  B2=PJ14  B3=PJ15  B4=PI4   B5=PI5   B6=PI6   B7=PI7
//!   G0=PJ7   G1=PJ8   G2=PJ9   G3=PJ10  G4=PJ11  G5=PK0   G6=PK1   G7=PK2
//!   R0=PI15  R1=PJ0   R2=PJ1   R3=PJ2   R4=PJ3   R5=PJ4   R6=PJ5   R7=PJ6
//!
//!   LCD_DISP    = PI12  (panel on, active-high)
//!   LCD_BL_CTRL = PK3   (backlight, active-high)
//!   LCD_DE      = PK7   (data-enable — wired on board but not passed to driver)

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::ltdc::{
    self, Ltdc, LtdcConfiguration, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals, Config};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// ---------------------------------------------------------------------------
// Display constants — RK043FN48H (480x272)
// ---------------------------------------------------------------------------
const LCD_WIDTH: u16 = 480;
const LCD_HEIGHT: u16 = 272;

const LCD_HSYNC: u16 = 41;
const LCD_HBP: u16 = 13;
const LCD_HFP: u16 = 32;
const LCD_VSYNC: u16 = 10;
const LCD_VBP: u16 = 2;
const LCD_VFP: u16 = 2;

// ---------------------------------------------------------------------------
// Static framebuffer — RGB565, 2 bytes/pixel, ~255 KB
// ---------------------------------------------------------------------------
const FB_SIZE: usize = LCD_WIDTH as usize * LCD_HEIGHT as usize;
static mut FRAMEBUFFER: [u16; FB_SIZE] = [0u16; FB_SIZE];

// ---------------------------------------------------------------------------
// Interrupt binding
// ---------------------------------------------------------------------------
bind_interrupts!(struct Irqs {
    LTDC => ltdc::InterruptHandler<peripherals::LTDC>;
});

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // -----------------------------------------------------------------------
    // Clock setup — 216 MHz from 25 MHz HSE via PLL
    // -----------------------------------------------------------------------
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;

        config.rcc.hse = Some(Hse {
            freq: Hertz(25_000_000),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll_src = PllSource::HSE;
        config.rcc.pll = Some(Pll {
            prediv: PllPreDiv::DIV25,  // VCO input = 1 MHz
            mul: PllMul::MUL432,       // VCO = 432 MHz
            divp: Some(PllPDiv::DIV2), // SYSCLK = 216 MHz
            divq: Some(PllQDiv::DIV9), // 48 MHz (USB)
            divr: None,
        });
        config.rcc.pllsai = Some(Pll {
            prediv: PllPreDiv::DIV25, // 25 MHz / 25 = 1 MHz
            mul: PllMul::MUL192,      // VCO = 192 MHz
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV4), // 192 / 4 = 48 MHz
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1; // HCLK  = 216 MHz
        config.rcc.apb1_pre = APBPrescaler::DIV4; // APB1  =  54 MHz
        config.rcc.apb2_pre = APBPrescaler::DIV2; // APB2  = 108 MHz
                                                  // PLLSAI for LTDC pixel clock is handled automatically by the HAL
    }

    let p = embassy_stm32::init(config);

    info!("STM32F746G-DISCO — embassy-stm32 0.6.0");

    // -----------------------------------------------------------------------
    // LCD enable pins (GPIO, not part of LTDC driver)
    // -----------------------------------------------------------------------
    let _lcd_disp = Output::new(p.PI12, Level::High, Speed::Low); // panel on
    let _lcd_bl = Output::new(p.PK3, Level::High, Speed::Low); // backlight on

    Timer::after(Duration::from_millis(20)).await;

    // -----------------------------------------------------------------------
    // LTDC timing (no background_color field in 0.6.0)
    // -----------------------------------------------------------------------
    let ltdc_config = LtdcConfiguration {
        pixel_clock_polarity: PolarityEdge::FallingEdge,
        h_sync_polarity: PolarityActive::ActiveLow,
        v_sync_polarity: PolarityActive::ActiveLow,
        data_enable_polarity: PolarityActive::ActiveHigh,

        h_sync: LCD_HSYNC - 1,
        h_back_porch: LCD_HBP - 1,
        active_width: LCD_WIDTH - 1,
        h_front_porch: LCD_HFP - 1,

        v_sync: LCD_VSYNC - 1,
        v_back_porch: LCD_VBP - 1,
        active_height: LCD_HEIGHT - 1,
        v_front_porch: LCD_VFP - 1,
    };

    // -----------------------------------------------------------------------
    // LTDC driver — embassy-stm32 0.6.0 signature (29 args):
    //   new_with_pins(peri, irqs, clk, hsync, vsync, b0..b7, g0..g7, r0..r7)
    //
    // Every pin below is the one that satisfies the corresponding
    // BnPin/GnPin/RnPin trait for stm32f746ng, confirmed from compiler errors.
    // -----------------------------------------------------------------------
    let mut display = Ltdc::new_with_pins(
        p.LTDC, Irqs, p.PI14, // CLK
        p.PI10, // HSYNC
        p.PI9,  // VSYNC
        // Blue:  B0      B1      B2      B3      B4     B5     B6     B7
        p.PE4, p.PJ13, p.PJ14, p.PJ15, p.PI4, p.PI5, p.PI6, p.PI7,
        // Green: G0      G1     G2      G3      G4      G5     G6     G7
        p.PJ7, p.PJ8, p.PJ9, p.PJ10, p.PJ11, p.PK0, p.PK1, p.PK2,
        // Red:   R0      R1     R2     R3     R4     R5     R6     R7
        p.PI15, p.PJ0, p.PJ1, p.PJ2, p.PJ3, p.PJ4, p.PJ5, p.PJ6,
    );

    // init() takes &LtdcConfiguration (not by value)
    display.init(&ltdc_config);

    // -----------------------------------------------------------------------
    // Layer: full-screen RGB565
    // -----------------------------------------------------------------------
    let layer_config = LtdcLayerConfig {
        pixel_format: PixelFormat::RGB565,
        layer: ltdc::LtdcLayer::Layer1,
        window_x0: 0,
        window_x1: LCD_WIDTH,
        window_y0: 0,
        window_y1: LCD_HEIGHT,
    };
    display.init_layer(&layer_config, None);

    info!("LTDC ready — entering render loop");

    // -----------------------------------------------------------------------
    // Render loop — scrolling three-band colour gradient
    // -----------------------------------------------------------------------
    let mut frame: u32 = 0;
    loop {
        // Fill framebuffer. SAFETY: only this task touches FRAMEBUFFER.
        unsafe { fill_gradient(&mut FRAMEBUFFER, frame) };

        // set_buffer takes *const () in 0.6.0
        unsafe {
            display
                .set_buffer(ltdc::LtdcLayer::Layer1, FRAMEBUFFER.as_ptr() as *const ())
                .await
                .ok();
        }

        frame = frame.wrapping_add(1);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pack 5-bit R, 6-bit G, 5-bit B into an RGB565 u16.
#[inline(always)]
const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0x1F) << 11) | ((g as u16 & 0x3F) << 5) | (b as u16 & 0x1F)
}

/// Scrolling three-band colour gradient.
fn fill_gradient(fb: &mut [u16; FB_SIZE], frame: u32) {
    let w = LCD_WIDTH as usize;
    let h = LCD_HEIGHT as usize;
    let shift = (frame as usize) % w;

    for y in 0..h {
        for x in 0..w {
            let sx = (x + shift) % w;
            let val = (sx * 63 / (w - 1)) as u8;

            fb[y * w + x] = if y < h / 3 {
                rgb565(val >> 1, 0, 0) // red
            } else if y < 2 * h / 3 {
                rgb565(0, val, 0) // green
            } else {
                rgb565(0, 0, val >> 1) // blue
            };
        }
    }

    // White border for easy verification
    for x in 0..w {
        fb[x] = rgb565(31, 63, 31);
        fb[(h - 1) * w + x] = rgb565(31, 63, 31);
    }
    for y in 0..h {
        fb[y * w] = rgb565(31, 63, 31);
        fb[y * w + w - 1] = rgb565(31, 63, 31);
    }
}
