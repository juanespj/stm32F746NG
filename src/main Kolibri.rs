//! Embassy LTDC + Kolibri GUI Example for STM32F746G-DISCO
//!
//! Kolibri is an immediate-mode GUI framework built on embedded-graphics.
//! It needs a DrawTarget — we provide one backed by our static framebuffer,
//! then flush the buffer to the display via LTDC each frame.
//!
//! Demonstrates:
//!   - Labels, Buttons, a counter, a Checkbox, a Slider
//!   - Smartstates (incremental redraw — only dirty widgets repaint)
//!   - Touch input stub (easily replaced with FT5336 I2C driver)

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::ltdc::{
    self, Ltdc, LtdcConfiguration, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, pac, peripherals, Config};
use embassy_time::{Duration, Timer};

use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};

use kolibri_embedded_gui::button::Button;
use kolibri_embedded_gui::checkbox::Checkbox;
use kolibri_embedded_gui::label::Label;
use kolibri_embedded_gui::slider::Slider;
use kolibri_embedded_gui::{
    prelude::*, smartstate::SmartstateProvider, style::medsize_rgb565_style, ui::Ui,
};

use {defmt_rtt as _, panic_probe as _};

// ---------------------------------------------------------------------------
// Display geometry
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
// Framebuffer in internal SRAM — RGB565, ~255 KB
// ---------------------------------------------------------------------------
const FB_SIZE: usize = LCD_WIDTH as usize * LCD_HEIGHT as usize;
static mut FRAMEBUFFER: [u16; FB_SIZE] = [0u16; FB_SIZE];

// ---------------------------------------------------------------------------
// DrawTarget wrapper
//
// Kolibri needs an embedded-graphics DrawTarget. We wrap our raw [u16]
// framebuffer in a thin struct that implements that trait.
// ---------------------------------------------------------------------------
struct FrameBuf {
    buf: &'static mut [u16; FB_SIZE],
}

impl FrameBuf {
    /// SAFETY: caller must ensure no other reference to FRAMEBUFFER exists.
    unsafe fn new() -> Self {
        Self {
            buf: &mut FRAMEBUFFER,
        }
    }
}

impl DrawTarget for FrameBuf {
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        for Pixel(pt, color) in pixels {
            if pt.x >= 0 && pt.x < LCD_WIDTH as i32 && pt.y >= 0 && pt.y < LCD_HEIGHT as i32 {
                let idx = pt.y as usize * LCD_WIDTH as usize + pt.x as usize;
                self.buf[idx] = color.into_storage();
            }
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let x0 = area.top_left.x.max(0) as usize;
        let y0 = area.top_left.y.max(0) as usize;
        let x1 = (area.top_left.x + area.size.width as i32).min(LCD_WIDTH as i32) as usize;
        let y1 = (area.top_left.y + area.size.height as i32).min(LCD_HEIGHT as i32) as usize;
        let raw = color.into_storage();
        for y in y0..y1 {
            for x in x0..x1 {
                self.buf[y * LCD_WIDTH as usize + x] = raw;
            }
        }
        Ok(())
    }
}

impl OriginDimensions for FrameBuf {
    fn size(&self) -> Size {
        Size::new(LCD_WIDTH as u32, LCD_HEIGHT as u32)
    }
}

// ---------------------------------------------------------------------------
// Interrupt binding
// ---------------------------------------------------------------------------
bind_interrupts!(struct Irqs {
    LTDC => ltdc::InterruptHandler<peripherals::LTDC>;
});

// ---------------------------------------------------------------------------
// Configure PLLSAI for LTDC pixel clock (~9.6 MHz)
// embassy-stm32 0.6.0 does not expose PLLSAI in rcc::Config for STM32F7,
// so we write the registers directly via the PAC after init().
//
//   PLLM = 25 (shared with main PLL, set by embassy)
//   PLLSAIN = 192  → VCO = 192 MHz
//   PLLSAIR = 5    → PLLSAI_R = 38.4 MHz
//   PLLSAIDIVR = /4 → pixel clock ≈ 9.6 MHz
// ---------------------------------------------------------------------------
fn configure_pllsai() {
    let rcc = pac::RCC;
    rcc.cr().modify(|w| w.set_pllsaion(false));
    while rcc.cr().read().pllsairdy() {}

    rcc.pllsaicfgr()
        .write(|w: &mut pac::rcc::regs::Pllsaicfgr| {
            w.set_plln(192.into()); // not set_pllsain
            w.set_pllq(2.into()); // not set_pllsaiq
            w.set_pllr(5.into()); // not set_pllsair
        });
    rcc.dckcfgr1().modify(|w| w.set_pllsaidivr(0b01.into())); // /4

    rcc.cr().modify(|w| w.set_pllsaion(true));
    while !rcc.cr().read().pllsairdy() {}
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // -----------------------------------------------------------------------
    // Clocks: 216 MHz system, PLLSAI configured below
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
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL432,
            divp: Some(PllPDiv::DIV2),
            divq: Some(PllQDiv::DIV9),
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV4;
        config.rcc.apb2_pre = APBPrescaler::DIV2;
    }

    let p = embassy_stm32::init(config);

    configure_pllsai();
    info!("PLLSAI locked");

    // -----------------------------------------------------------------------
    // LCD power + backlight
    // -----------------------------------------------------------------------
    let _lcd_disp = Output::new(p.PI12, Level::High, Speed::Low);
    let _lcd_bl = Output::new(p.PK3, Level::High, Speed::Low);
    Timer::after(Duration::from_millis(20)).await;

    // -----------------------------------------------------------------------
    // LTDC
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

    let mut display = Ltdc::new_with_pins(
        p.LTDC, Irqs, p.PI14, p.PI10, p.PI9, p.PE4, p.PJ13, p.PJ14, p.PJ15, p.PI4, p.PI5, p.PI6,
        p.PI7, p.PJ7, p.PJ8, p.PJ9, p.PJ10, p.PJ11, p.PK0, p.PK1, p.PK2, p.PI15, p.PJ0, p.PJ1,
        p.PJ2, p.PJ3, p.PJ4, p.PJ5, p.PJ6,
    );
    display.init(&ltdc_config);
    display.init_layer(
        &LtdcLayerConfig {
            pixel_format: PixelFormat::RGB565,
            layer: ltdc::LtdcLayer::Layer1,
            window_x0: 0,
            window_x1: LCD_WIDTH,
            window_y0: 0,
            window_y1: LCD_HEIGHT,
        },
        None,
    );

    info!("LTDC ready");

    // -----------------------------------------------------------------------
    // Application state
    // -----------------------------------------------------------------------
    let mut counter: i32 = 0;
    let mut checked = false;
    let mut slider_val: i16 = 50;

    // Kolibri smartstates: one per widget that should use incremental redraw.
    // We use 10 slots — adjust if you add/remove widgets.
    let mut smartstates = SmartstateProvider::<10>::new();

    // Small widget draw buffer (avoids flicker without a full second framebuffer)
    let mut widget_buf = [Rgb565::BLACK; 120 * 40];

    // Wrap the static framebuffer in our DrawTarget
    // SAFETY: only this task ever touches FRAMEBUFFER.
    let mut fb = unsafe { FrameBuf::new() };

    // Clear the screen once before entering the loop
    fb.clear(Rgb565::BLACK).ok();

    // -----------------------------------------------------------------------
    // Render loop
    // -----------------------------------------------------------------------
    loop {
        smartstates.restart_counter();

        // Build the Kolibri UI each frame.
        // Kolibri draws directly into `fb` (our framebuffer-backed DrawTarget).
        let mut ui = Ui::new_fullscreen(&mut fb, medsize_rgb565_style());
        ui.set_buffer(&mut widget_buf);

        // --- Title ---
        ui.add(Label::new("STM32F746G-DISCO").smartstate(smartstates.nxt()));

        // --- Counter row ---
        ui.add(Label::new("Counter:").smartstate(smartstates.nxt()));

        if ui
            .add_horizontal(Button::new(" - ").smartstate(smartstates.nxt()))
            .clicked()
        {
            counter = counter.saturating_sub(1);
        }

        // Use a fixed-width string to avoid layout shifts
        let mut count_str = heapless::String::<16>::new();
        core::fmt::write(&mut count_str, format_args!("{:>6}", counter)).ok();
        ui.add_horizontal(Label::new(count_str.as_str()).smartstate(smartstates.nxt()));

        if ui
            .add_horizontal(Button::new(" + ").smartstate(smartstates.nxt()))
            .clicked()
        {
            counter = counter.saturating_add(1);
        }

        // --- Checkbox ---
        ui.add(Checkbox::new(&mut checked).smartstate(smartstates.nxt()));
        ui.add_horizontal(
            Label::new(if checked { "Enabled" } else { "Disabled" }).smartstate(smartstates.nxt()),
        );

        // --- Slider ---
        ui.add(Label::new("Brightness:").smartstate(smartstates.nxt()));
        ui.add(Slider::new(&mut slider_val, 0..=100).smartstate(smartstates.nxt()));

        // --- Flush framebuffer to LCD ---
        // SAFETY: fb.buf points into FRAMEBUFFER which we own exclusively.
        display
            .set_buffer(ltdc::LtdcLayer::Layer1, fb.buf.as_ptr() as *const ())
            .await
            .ok();

        // Small yield so the executor can handle other tasks
        Timer::after(Duration::from_millis(16)).await; // ~60 fps cap
    }
}
