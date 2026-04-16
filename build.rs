use std::{fs::File, io::Write, path::Path};

fn main() {
    // Re-run if images change
    println!("cargo:rerun-if-changed=assets/");

    convert_image("assets/img.png", "assets/img.raw");
    convert_image("assets/bot.png", "assets/bot.raw");
}

fn convert_image(input: &str, output: &str) {
    let img = image::open(input).expect("Failed to open image");

    // Convert to RGB8
    let img = img.to_rgb8();
    let (width, height) = img.dimensions();

    let mut out = File::create(output).expect("Failed to create raw file");

    for pixel in img.pixels() {
        let [r, g, b] = pixel.0;

        let rgb565 = ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | ((b as u16) >> 3);
        // let rgb565 = ((255 as u16 & 0xF8) << 8) |      // force RED = 0
        // ((0 as u16 & 0xFC) << 3) |      // force GREEN = 0
        // ((0 as u16) >> 3); // BLUE = max

        // let rgb565 = ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | ((b as u16) >> 3);
        // little endian

        // out.write_all(&[(rgb565 & 0xFF) as u8, (rgb565 >> 8) as u8])
        //     .unwrap();
        //big endian
        out.write_all(&[(rgb565 >> 8) as u8, (rgb565 & 0xFF) as u8])
            .unwrap();
    }

    println!("Converted {} ({}x{}) -> {}", input, width, height, output);
}

// fn draw_background(fb: &mut FrameBuf) {
//     let img: &[u8] = include_bytes!("../assets/bg.raw");

//     let mut i = 0;

//     for y in 0..LCD_HEIGHT as usize {
//         for x in 0..LCD_WIDTH as usize {
//             let lo = img[i] as u16;
//             let hi = img[i + 1] as u16;
//             let color = (hi << 8) | lo;

//             fb.buf[y * LCD_WIDTH as usize + x] = color;
//             i += 2;
//         }
//     }
// }
