//! Embassy LTDC Display Example for STM32F746G-DISCO
//!
//! Targets embassy-stm32 0.6.0 / embassy-executor 0.10.0 / embassy-time 0.5.1
//!
//! The LTDC driver asserts that the LTDC GCR register reads 0x2220 on init,
//! which means PLLSAI must be running and supplying the pixel clock BEFORE
//! display.init() is called.
//!
//! embassy-stm32 0.6.0 does not expose PLLSAI configuration in rcc::Config
//! for STM32F7, so we configure it directly via the PAC after embassy_stm32::init().
//!
//! PLLSAI target: ~9.6 MHz pixel clock for the RK043FN48H
//!   VCO input = PLLM = 25 (shared with main PLL, 25 MHz HSE / 25 = 1 MHz)
//!   PLLSAIN   = 192  → VCO = 192 MHz
//!   PLLSAIR   = 5    → PLLSAI_R = 38.4 MHz
//!   LTDC divider (PLLSAIDIVR) = /4  → pixel clock = 9.6 MHz ✓

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::ltdc::{
    self, Ltdc, LtdcConfiguration, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::pac::rcc::regs::Pllsaicfgr;
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, pac, peripherals, Config};
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _};

// ---------------------------------------------------------------------------
// Display constants — RK043FN48H (480x272, RGB565)
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
// Framebuffer in internal SRAM — 480*272*2 = 261120 bytes (~255 KB)
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
// Configure PLLSAI for LTDC pixel clock via PAC
//
// Called AFTER embassy_stm32::init() so the main PLL (and PLLM) is already
// locked.  PLLSAI shares PLLM with the main PLL.
//
// Register layout (RM0385 section 5.3.23, RCC_PLLSAICFGR):
//   Bits [14:6]  PLLSAIN  multiplication factor (50..432)
//   Bits [30:28] PLLSAIR  division factor for LTDC (2..7)
//   Bits [27:24] PLLSAIQ  (not used, set to 2 as minimum valid value)
//
// RCC_DCKCFGR1 bits [17:16] PLLSAIDIVR: extra LTDC divider
//   00 = /2   01 = /4   10 = /8   11 = /16
//
// We want: VCO in = 1 MHz, N=192 → VCO=192 MHz, R=5 → 38.4 MHz, /4 → 9.6 MHz
// ---------------------------------------------------------------------------
fn configure_pllsai() {
    let rcc = pac::RCC;

    // Disable PLLSAI before modifying
    rcc.cr().modify(|w| w.set_pllsaion(false));
    while rcc.cr().read().pllsairdy() {}

    // Set PLLSAIN=192, PLLSAIQ=2 (min), PLLSAIR=5
    rcc.pllsaicfgr().write(|w: &mut Pllsaicfgr| {
        w.set_pllsain(192.into());
        w.set_pllsaiq(2);
        w.set_pllsair(5);
    });

    // Set LTDC extra divider to /4 (PLLSAIDIVR = 0b01)
    rcc.dckcfgr1().modify(|w| w.set_pllsaidivr(0b01));

    // Enable PLLSAI and wait for lock
    rcc.cr().modify(|w| w.set_pllsaion(true));
    while !rcc.cr().read().pllsairdy() {}
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // -----------------------------------------------------------------------
    // Main clock: 216 MHz from 25 MHz HSE
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
            prediv: PllPreDiv::DIV25,  // VCO input = 1 MHz (PLLM=25, shared)
            mul: PllMul::MUL432,       // VCO = 432 MHz
            divp: Some(PllPDiv::DIV2), // SYSCLK = 216 MHz
            divq: Some(PllQDiv::DIV9), // 48 MHz (USB)
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV4;
        config.rcc.apb2_pre = APBPrescaler::DIV2;
    }

    let p = embassy_stm32::init(config);

    info!("Boot OK — configuring PLLSAI for LTDC pixel clock");

    // Configure PLLSAI now that the main PLL (and shared PLLM) is locked
    configure_pllsai();

    info!("PLLSAI locked — pixel clock ~9.6 MHz");

    // -----------------------------------------------------------------------
    // LCD enable pins
    // -----------------------------------------------------------------------
    let _lcd_disp = Output::new(p.PI12, Level::High, Speed::Low);
    let _lcd_bl = Output::new(p.PK3, Level::High, Speed::Low);

    Timer::after(Duration::from_millis(20)).await;

    // -----------------------------------------------------------------------
    // LTDC configuration
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
    // LTDC driver — 29 args: peri, irqs, clk, hsync, vsync, b0..b7, g0..g7, r0..r7
    // All pins verified from embassy 0.6.0 trait impls for stm32f746ng.
    // -----------------------------------------------------------------------
    let mut display = Ltdc::new_with_pins(
        p.LTDC, Irqs, p.PI14, // CLK
        p.PI10, // HSYNC
        p.PI9,  // VSYNC
        // Blue  b0     b1      b2      b3      b4     b5     b6     b7
        p.PE4, p.PJ13, p.PJ14, p.PJ15, p.PI4, p.PI5, p.PI6, p.PI7,
        // Green g0     g1     g2      g3      g4      g5     g6     g7
        p.PJ7, p.PJ8, p.PJ9, p.PJ10, p.PJ11, p.PK0, p.PK1, p.PK2,
        // Red   r0      r1     r2     r3     r4     r5     r6     r7
        p.PI15, p.PJ0, p.PJ1, p.PJ2, p.PJ3, p.PJ4, p.PJ5, p.PJ6,
    );

    // init() takes &LtdcConfiguration
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

    info!("LTDC ready — render loop starting");

    // -----------------------------------------------------------------------
    // Render loop
    // -----------------------------------------------------------------------
    let mut frame: u32 = 0;
    loop {
        let fb = unsafe { &mut FRAMEBUFFER };
        fill_gradient(fb, frame);

        display
            .set_buffer(ltdc::LtdcLayer::Layer1, fb.as_ptr() as *const ())
            .await
            .ok();

        frame = frame.wrapping_add(1);
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

#[inline(always)]
const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0x1F) << 11) | ((g as u16 & 0x3F) << 5) | (b as u16 & 0x1F)
}

fn fill_gradient(fb: &mut [u16; FB_SIZE], frame: u32) {
    let w = LCD_WIDTH as usize;
    let h = LCD_HEIGHT as usize;
    let shift = (frame as usize) % w;

    for y in 0..h {
        for x in 0..w {
            let sx = (x + shift) % w;
            let val = (sx * 63 / (w - 1)) as u8;
            fb[y * w + x] = if y < h / 3 {
                rgb565(val >> 1, 0, 0)
            } else if y < 2 * h / 3 {
                rgb565(0, val, 0)
            } else {
                rgb565(0, 0, val >> 1)
            };
        }
    }
    // White border
    for x in 0..w {
        fb[x] = rgb565(31, 63, 31);
        fb[(h - 1) * w + x] = rgb565(31, 63, 31);
    }
    for y in 0..h {
        fb[y * w] = rgb565(31, 63, 31);
        fb[y * w + w - 1] = rgb565(31, 63, 31);
    }
}
