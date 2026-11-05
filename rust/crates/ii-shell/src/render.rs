use tiny_skia::{Color, Paint, PixmapMut, Rect, Transform};

/// Draws the idle panel background and a restrained accent marker into a
/// caller-owned Wayland SHM buffer. No intermediate full-frame allocation.
pub fn draw_panel(pixels: &mut [u8], width: u32, height: u32) {
    let Some(mut pixmap) = PixmapMut::from_bytes(pixels, width, height) else {
        return;
    };

    pixmap.fill(Color::from_rgba8(20, 19, 19, 255));

    let Some(rect) = Rect::from_xywh(0.0, 0.0, 3.0, height as f32) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(178, 83, 93, 255));
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_supplied_buffer_without_changing_its_size() {
        let width = 20;
        let height = 8;
        let mut pixels = vec![0; width * height * 4];
        draw_panel(&mut pixels, width as u32, height as u32);

        assert_eq!(pixels.len(), width * height * 4);
        assert_eq!(&pixels[0..4], &[20, 19, 19, 255]);
    }
}
