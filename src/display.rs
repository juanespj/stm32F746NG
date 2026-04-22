use embassy_stm32::ltdc::{LtdcConfiguration, PolarityActive, PolarityEdge};
use embassy_stm32::pac;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
// ---------------------------------------------------------------------------
// Display geometry
// ---------------------------------------------------------------------------
pub const LCD_WIDTH: u16 = 480;
pub const LCD_HEIGHT: u16 = 272;
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
// PLLSAI for LTDC pixel clock (~9.6 MHz) via PAC
// ---------------------------------------------------------------------------
pub fn configure_pllsai() {
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

// ---------------------------------------------------------------------------
// DrawTarget — wraps the framebuffer for Kolibri / embedded-graphics
// ---------------------------------------------------------------------------
pub struct FrameBuf {
    pub buf: &'static mut [u16; FB_SIZE],
}

impl FrameBuf {
    pub unsafe fn new() -> Self {
        Self {
            buf: unsafe { &mut *(&raw mut FRAMEBUFFER) },
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

pub fn ltdccfg() -> LtdcConfiguration {
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
    ltdc_config
}

pub struct Display {
    pub fb: FrameBuf,
}

impl Display {
    pub fn init(/* ltdc, pins */) -> Self {
        let mut fb = unsafe { FrameBuf::new() };
        fb.clear(Rgb565::BLACK).ok();

        Self { fb }
    }
}
