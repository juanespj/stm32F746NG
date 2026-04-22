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
mod display;
mod touch;
mod ui;
use crate::display::ltdccfg;
use crate::ui::ui_task;
use defmt::info;
use display::{configure_pllsai, FrameBuf, LCD_HEIGHT, LCD_WIDTH};
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::{self, I2c, Master};

use embassy_stm32::ltdc::{self, Ltdc, LtdcLayerConfig, PixelFormat};
use embassy_stm32::time::Hertz;
use embassy_stm32::{bind_interrupts, peripherals, Config};
use embassy_time::{Delay, Duration, Timer};
use ft5336::Ft5336;
use touch::touch_task;
use {defmt_rtt as _, panic_probe as _};

// ---------------------------------------------------------------------------
// Interrupt bindings
// ---------------------------------------------------------------------------
bind_interrupts!(struct Irqs {
    LTDC => ltdc::InterruptHandler<peripherals::LTDC>;
    I2C3_EV => i2c::EventInterruptHandler<peripherals::I2C3>;
    I2C3_ER => i2c::ErrorInterruptHandler<peripherals::I2C3>;
});

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
#[embassy_executor::main]
async fn main(spawner: Spawner) {
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

    // let mut i2c_bus = BlockingI2c { inner: i2c };

    // // Delay source required by ft5336::Ft5336::new()
    // let mut delay = Delay;

    // // Initialise the FT5336 driver
    // let mut touch = Ft5336::new(&i2c_bus, FT5336_ADDR, &mut delay)
    //     .expect("FT5336 init failed — check I2C wiring");
    // touch.init(&mut i2c_bus);
    spawner.spawn(touch_task(p.I2C3, p.PH7, p.PH8, touch_int).unwrap());
    info!("FT5336 touch controller ready");

    let ltdc_config = ltdccfg();

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
    // let mut fb = unsafe { FrameBuf::new() };
    // -----------------------------------------------------------------------
    // Application state
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------

    spawner.spawn(ui_task(display).unwrap());
    loop {
        // -------------------------------------------------------------------
        Timer::after_millis(100).await;
    }
}
