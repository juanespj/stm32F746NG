use crate::touch::TOUCH_CH;
use crate::FrameBuf;
use defmt::info;
use embassy_stm32::ltdc::{self, Ltdc};
use embassy_stm32::peripherals::LTDC;
use embedded_graphics::image::{Image, ImageRaw};
use embedded_graphics::mono_font::ascii;
use embedded_graphics::{pixelcolor::Rgb565, prelude::*, primitives::Rectangle};
use kolibri_embedded_gui::button::Button;
use kolibri_embedded_gui::checkbox::Checkbox;
use kolibri_embedded_gui::label::Label;
use kolibri_embedded_gui::slider::Slider;
use kolibri_embedded_gui::style::medsize_rgb565_style; // medsize_blue_rgb565_style, medsize_crt_rgb565_style, medsize_light_rgb565_style,medsize_sakura_rgb565_stylemedsize_retro_rgb565_style
use kolibri_embedded_gui::ui::Interaction;
use kolibri_embedded_gui::{smartstate::SmartstateProvider, ui::Ui};
// 2x2 RGB565 raw image
// const IMAGE_RAW: &[u8] = &[0xF8, 0x00, 0xF8, 0x00, 0xF8, 0x00, 0xF8, 0x00];
const IMAGE_DATA: &[u8] = include_bytes!("../assets/img.raw");
const BOT_DATA: &[u8] = include_bytes!("../assets/bot.raw");
#[embassy_executor::task]
pub async fn ui_task(mut display: Ltdc<'static, LTDC>) {
    let mut counter: i32 = 0;
    let mut checked = false;
    let mut slider_val: i16 = 50;
    let mut smartstates = SmartstateProvider::<10>::new();
    let mut widget_buf = [Rgb565::BLACK; 120 * 40];
    let mut fb = unsafe { FrameBuf::new() };

    fb.clear(Rgb565::BLACK).ok();

    // -----------------------------------------------------------------------
    // Render + touch loop

    let raw = ImageRaw::<Rgb565>::new(BOT_DATA, 212);
    Image::new(&raw, Point::new(200, 50)).draw(&mut fb).unwrap();
    loop {
        let event = if let Ok(event) = TOUCH_CH.try_receive() {
            // interaction = match event {
            //     TouchEvent::Press(p) => Interaction::Click(p),
            //     TouchEvent::Move(p) => Interaction::Drag(p),
            //     TouchEvent::Release(p) => Interaction::Release(p),
            // };
            event
        } else {
            Interaction::None
        };

        // Drain all pending touch events

        // Build UI
        smartstates.restart_counter();
        let mut style = medsize_rgb565_style();

        // Change text color
        style.text_color = Rgb565::RED;

        // Optional:
        style.background_color = Rgb565::BLACK;
        style.border_color = Rgb565::WHITE;
        let mut ui = Ui::new_fullscreen(&mut fb, style);

        ui.set_buffer(&mut widget_buf);
        // Feed touch input into Kolibri BEFORE adding widgets
        ui.interact(event);

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

        // IMPORTANT: reset interaction after frame
    }
}
