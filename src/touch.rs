use embassy_stm32::gpio::{Input, Level, Output, Pull, Speed};
use embassy_stm32::i2c::Config as I2cConfig;
use embassy_stm32::i2c::{self, I2c, Master};
use embassy_stm32::time::Hertz;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Delay;
use embassy_time::Timer;
use embedded_graphics::prelude::Point;
use ft5336::Ft5336;
use kolibri_embedded_gui::ui::Interaction;
// FT5336 I2C address (fixed in hardware)
pub const FT5336_ADDR: u8 = 0x38;
pub static TOUCH_CH: Channel<ThreadModeRawMutex, Interaction, 4> = Channel::new();
use embassy_stm32::{bind_interrupts, peripherals, Config};

pub struct TouchHandler {
    pub was_touching: bool,
    last_point: Point,

    stable_count: u8,
    release_count: u8,
}

impl TouchHandler {
    pub fn new() -> Self {
        Self {
            was_touching: false,
            last_point: Point::zero(),
            stable_count: 0,
            release_count: 0,
        }
    }

    pub fn update(&mut self, touch_detected: bool, touch_point: Option<(i32, i32)>) -> Interaction {
        const PRESS_THRESHOLD: u8 = 2; // frames to confirm press
        const RELEASE_THRESHOLD: u8 = 3; // frames to confirm release

        let mut interaction = Interaction::None;

        let current_point = if let Some((x, y)) = touch_point {
            Point::new(x, y)
        } else {
            self.last_point
        };

        if touch_detected {
            self.release_count = 0;

            if self.stable_count < PRESS_THRESHOLD {
                self.stable_count += 1;
            }

            if !self.was_touching && self.stable_count >= PRESS_THRESHOLD {
                // Confirmed new press
                self.was_touching = true;
                interaction = Interaction::Click(current_point);
            } else if self.was_touching {
                // Stable hold
                interaction = Interaction::Drag(current_point);
            }
        } else {
            self.stable_count = 0;

            if self.was_touching {
                if self.release_count < RELEASE_THRESHOLD {
                    self.release_count += 1;
                }

                if self.release_count >= RELEASE_THRESHOLD {
                    // Confirmed release
                    self.was_touching = false;
                    interaction = Interaction::Release(self.last_point);
                }
            }
        }

        self.last_point = current_point;
        interaction
    }
}

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
use embassy_stm32::Peri;

#[embassy_executor::task]
pub async fn touch_task(
    i2c: Peri<'static, peripherals::I2C3>,
    scl: Peri<'static, peripherals::PH7>,
    sda: Peri<'static, peripherals::PH8>,
    mut int_pin: Input<'static>,
) {
    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = Hertz(100_000);
    let i2c = I2c::new_blocking(i2c, scl, sda, i2c_config);
    let mut i2c_bus = BlockingI2c { inner: i2c };

    let mut delay = Delay;

    let mut touch = Ft5336::new(&i2c_bus, FT5336_ADDR, &mut delay).expect("FT5336 init failed");

    touch.init(&mut i2c_bus);

    let mut handler = TouchHandler::new();

    loop {
        let detected = int_pin.is_low();

        let point = if detected {
            touch
                .get_touch(&mut i2c_bus, detected as u8)
                .ok()
                .map(|t| (t.y as i32, t.x as i32))
        } else {
            None
        };

        let interaction = handler.update(detected, point);

        if interaction != Interaction::None {
            TOUCH_CH.send(interaction).await;
        }
        Timer::after_millis(50).await;
    }
}
