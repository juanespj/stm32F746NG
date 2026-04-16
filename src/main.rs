//! Embassy LTDC + Kolibri GUI + FT5336 Touch — STM32F746G-DISCO
//!
//! Hardware:
//!   Display: RK043FN48H 480x272 RGB565 via LTDC
//!   Touch:   FT5336 capacitive controller via I2C3
//!              SDA = PH8,  SCL = PH7,  INT = PI13
//!
//! The `ft5336` crate uses embedded-hal 0.2 blocking I2C.
//! embassy-stm32 implements those traits via the `embedded-hal-02` compat layer,
//! so we create a blocking I2C wrapper around embassy's async I2c.
//!
//! Kolibri input API (kolibri-embedded-gui 0.1.0):
//!   ui.interact(InputType::Touch(Point::new(x, y)))  — finger down / moving
//!   ui.interact(InputType::None)                      — no touch / finger up

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::{self, I2c, Master};
use embassy_stm32::ltdc::{
    self, Ltdc, LtdcConfiguration, LtdcLayerConfig, PixelFormat, PolarityActive, PolarityEdge,
};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, pac, peripherals, Config};
use embassy_time::{Delay, Duration, Timer};
use embedded_graphics::image::{Image, ImageRaw};
use embedded_graphics::mono_font::ascii;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use ft5336::Ft5336;
use kolibri_embedded_gui::button::Button;
use kolibri_embedded_gui::checkbox::Checkbox;
use kolibri_embedded_gui::label::Label;
use kolibri_embedded_gui::slider::Slider;
use kolibri_embedded_gui::style::medsize_rgb565_style; // medsize_blue_rgb565_style, medsize_crt_rgb565_style, medsize_light_rgb565_style,medsize_sakura_rgb565_stylemedsize_retro_rgb565_style
use kolibri_embedded_gui::{smartstate::SmartstateProvider, ui::Ui};
use {defmt_rtt as _, panic_probe as _};
mod touch;
use touch::TouchHandler;
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

// FT5336 I2C address (fixed in hardware)
const FT5336_ADDR: u8 = 0x38;

// ---------------------------------------------------------------------------
// Framebuffer in internal SRAM — RGB565, ~255 KB
// ---------------------------------------------------------------------------
const FB_SIZE: usize = LCD_WIDTH as usize * LCD_HEIGHT as usize;
static mut FRAMEBUFFER: [u16; FB_SIZE] = [0u16; FB_SIZE];

// ---------------------------------------------------------------------------
// Blocking I2C wrapper for the ft5336 crate
//
// embassy-stm32's I2c is async, but ft5336 needs embedded-hal 0.2 blocking
// I2C traits.  We implement those traits by blocking on the async operations.
// In an embassy context, this is fine for a peripheral polled at 60 fps —
// the I2C transfer completes in microseconds.
// ---------------------------------------------------------------------------
struct BlockingI2c<'d> {
    inner: I2c<'d, embassy_stm32::mode::Blocking, Master>,
}

impl embedded_hal::blocking::i2c::Write for BlockingI2c<'_> {
    type Error = i2c::Error;
    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        self.inner.blocking_write(addr, bytes)
    }
}

impl embedded_hal::blocking::i2c::WriteRead for BlockingI2c<'_> {
    type Error = i2c::Error;
    fn write_read(&mut self, addr: u8, bytes: &[u8], buf: &mut [u8]) -> Result<(), Self::Error> {
        self.inner.blocking_write_read(addr, bytes, buf)
    }
}

// ---------------------------------------------------------------------------
// DrawTarget — wraps the framebuffer for Kolibri / embedded-graphics
// ---------------------------------------------------------------------------
struct FrameBuf {
    buf: &'static mut [u16; FB_SIZE],
}

impl FrameBuf {
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
                self.buf[pt.y as usize * LCD_WIDTH as usize + pt.x as usize] = color.into_storage();
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
            self.buf[y * LCD_WIDTH as usize + x0..y * LCD_WIDTH as usize + x1].fill(raw);
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
// Interrupt bindings
// ---------------------------------------------------------------------------
bind_interrupts!(struct Irqs {
    LTDC => ltdc::InterruptHandler<peripherals::LTDC>;
    I2C3_EV => i2c::EventInterruptHandler<peripherals::I2C3>;
    I2C3_ER => i2c::ErrorInterruptHandler<peripherals::I2C3>;
});

// ---------------------------------------------------------------------------
// PLLSAI for LTDC pixel clock (~9.6 MHz) via PAC
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
    rcc.dckcfgr1().modify(|w| w.set_pllsaidivr(0b01_u8.into())); // /4
    rcc.cr().modify(|w| w.set_pllsaion(true));
    while !rcc.cr().read().pllsairdy() {}
}

// 2x2 RGB565 raw image
// const IMAGE_RAW: &[u8] = &[0xF8, 0x00, 0xF8, 0x00, 0xF8, 0x00, 0xF8, 0x00];
const IMAGE_DATA: &[u8] = include_bytes!("../assets/img.raw");
const BOT_DATA: &[u8] = include_bytes!("../assets/bot.raw");

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // -----------------------------------------------------------------------
    // Clocks: 216 MHz system clock
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
    info!("Clocks OK");

    // -----------------------------------------------------------------------
    // LCD power + backlight
    // -----------------------------------------------------------------------
    let _lcd_disp = Output::new(p.PI12, Level::High, Speed::Low);
    let _lcd_bl = Output::new(p.PK3, Level::High, Speed::Low);

    // Touch INT pin — active-low, driven by FT5336 when a touch is detected.
    // Reading it lets us skip I2C polling when there's no touch.
    let touch_int = Input::new(p.PI13, Pull::Up);

    Timer::after(Duration::from_millis(20)).await;

    // -----------------------------------------------------------------------
    // I2C3 for FT5336 touch controller
    // SDA = PH8 (AF4), SCL = PH7 (AF4)
    // -----------------------------------------------------------------------

    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = Hertz(100_000);

    let i2c = I2c::new_blocking(p.I2C3, p.PH7, p.PH8, i2c_config);
    let mut i2c_bus = BlockingI2c { inner: i2c };

    // Delay source required by ft5336::Ft5336::new()
    let mut delay = Delay;

    // Initialise the FT5336 driver
    let mut touch = Ft5336::new(&i2c_bus, FT5336_ADDR, &mut delay)
        .expect("FT5336 init failed — check I2C wiring");
    touch.init(&mut i2c_bus);

    info!("FT5336 touch controller ready");

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
        p.LTDC, Irqs, p.PI14, p.PI10, p.PI9, p.PE4, p.PJ13, p.PJ14, p.PJ15, p.PG12, p.PK4, p.PK5,
        p.PK6, p.PJ7, p.PJ8, p.PJ9, p.PJ10, p.PJ11, p.PK0, p.PK1, p.PK2, p.PI15, p.PJ0, p.PJ1,
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
    let mut smartstates = SmartstateProvider::<10>::new();
    let mut widget_buf = [Rgb565::BLACK; 120 * 40];
    let mut fb = unsafe { FrameBuf::new() };
    let mut touch_handler = TouchHandler::new();
    fb.clear(Rgb565::BLACK).ok();
    let raw = ImageRaw::<Rgb565>::new(BOT_DATA, 212);
    Image::new(&raw, Point::new(200, 50)).draw(&mut fb).unwrap();
    // -----------------------------------------------------------------------
    // Render + touch loop
    // -----------------------------------------------------------------------
    loop {
        // -------------------------------------------------------------------
        // Read touch state
        //
        // Poll the INT pin first — it goes low when the FT5336 has data.
        // This avoids unnecessary I2C traffic on most frames.
        // -------------------------------------------------------------------
        let touch_detected = if touch_int.is_low() {
            match touch.detect_touch(&mut i2c_bus) {
                Ok(n) => n > 0,
                Err(_) => touch_handler.was_touching, // Keep previous state on I2C error
            }
        } else {
            false
        };

        let touch_point = if touch_detected {
            match touch.get_touch(&mut i2c_bus, 1) {
                Ok(t) => {
                    // IMPORTANT: adjust mapping here if needed
                    let (x, y) = (t.y as i32, t.x as i32);

                    //draw point at touch position!("Touch at ({}, {})", x, y);
                    // fb.buf[(y as usize * LCD_WIDTH as usize + x as usize)] = 0xFFFF;

                    Some((x, y))
                }
                Err(_) => None,
            }
        } else {
            None
        };

        let input = touch_handler.update(touch_detected, touch_point);
        // match input {
        //     Interaction::Click(_) => info!("CLICK"),
        //     Interaction::Drag(_) => info!("DRAG"),
        //     Interaction::Release(_) => info!("RELEASE"),
        //     _ => {}
        // }
        // -------------------------------------------------------------------
        // Build Kolibri UI
        // -------------------------------------------------------------------

        let mut style = medsize_rgb565_style();

        // Change text color
        style.text_color = Rgb565::RED;

        // Optional:
        style.background_color = Rgb565::BLACK;
        style.border_color = Rgb565::WHITE;
        smartstates.restart_counter(); //In Kolibri, Smartstate is used to skip redrawing widgets that haven't changed to save CPU cycles. If the UI thinks the "Label" hasn't changed, it won't push the new pixels to your framebuffer.

        let mut ui = Ui::new_fullscreen(&mut fb, style);

        ui.set_buffer(&mut widget_buf);
        // Feed touch input into Kolibri BEFORE adding widgets
        ui.interact(input);

        // Title
        ui.add(Label::new("STM32F746G-DISCO + Touch").smartstate(smartstates.nxt()));
        let mut count_str = heapless::String::<16>::new();
        core::fmt::write(&mut count_str, format_args!("{}", counter)).ok();

        // Counter buttons
        ui.add(Label::new("Counter:").smartstate(smartstates.nxt()));
        if ui
            .add_horizontal(Button::new(" - ").smartstate(smartstates.nxt()))
            .clicked()
        {
            counter = counter.saturating_sub(1);
            info!("Counter decremented: {}", counter);
        }
        ui.add_horizontal(Label::new(count_str.as_str())); // No smartstate

        // ui.add_horizontal(Label::new(count_str.as_str()).smartstate(smartstates.nxt()));

        if ui
            .add_horizontal(Button::new(" + ").smartstate(smartstates.nxt()))
            .clicked()
        {
            counter = counter.saturating_add(1);
            info!("Counter incremented: {}", counter);
        }
        let style = ui.style_mut();
        style.text_color = Rgb565::BLUE;

        ui.add_horizontal(Label::new("Theming Example").with_font(ascii::FONT_10X20));
        // Checkbox
        ui.add(Checkbox::new(&mut checked).smartstate(smartstates.nxt()));
        ui.add_horizontal(
            Label::new(if checked { "Enabled" } else { "Disabled" }).smartstate(smartstates.nxt()),
        );

        // Slider
        ui.add(Label::new("Brightness:").smartstate(smartstates.nxt()));

        ui.add(Slider::new(&mut slider_val, 0..=100).smartstate(smartstates.nxt()));

        // Example raw image (you must provide bytes)
        // let raw: ImageRaw<Rgb565> = ImageRaw::new(include_bytes!("my_image.raw"), 100);
        // const IMAGE_RAW: &[u16] = &[0xF800; 400];
        // let raw = ImageRaw::<Rgb565>::new(IMAGE_RAW, 100);

        // let raw: ImageRaw<Rgb565> = ImageRaw::new(include_bytes!("my_image.raw"), 100);

        // -------------------------------------------------------------------
        // Flush framebuffer to display (waits for vsync)
        // -------------------------------------------------------------------
        display
            .set_buffer(ltdc::LtdcLayer::Layer1, fb.buf.as_ptr() as *const ())
            .await
            .ok();
    }
}
