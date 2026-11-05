use cosmic_text::{Attrs, Buffer, Color, FontSystem, Metrics, Shaping, SwashCache};

/// One process-wide font database and glyph cache. Widgets borrow this engine
/// instead of constructing independent font systems and raster caches.
pub struct TextEngine {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl TextEngine {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    /// Shapes and composites text into the caller's premultiplied RGBA buffer.
    /// The callback clips every glyph span to the supplied surface bounds.
    pub fn draw_text(
        &mut self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        text: &str,
        origin: (i32, i32),
        font_size: f32,
    ) {
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.35),
        );
        let mut buffer = buffer.borrow_with(&mut self.font_system);
        buffer.set_size(
            Some(width.saturating_sub(origin.0.max(0) as u32) as f32),
            Some(height.saturating_sub(origin.1.max(0) as u32) as f32),
        );
        buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);

        let mut target = TextCanvas {
            pixels,
            width,
            height,
            origin,
        };
        buffer.draw(
            &mut self.swash_cache,
            Color::rgba(235, 231, 232, 255),
            |x, y, span_width, span_height, color| {
                target.blend_span(x, y, span_width, span_height, color);
            },
        );
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

struct TextCanvas<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    origin: (i32, i32),
}

impl TextCanvas<'_> {
    fn blend_span(
        &mut self,
        x: i32,
        y: i32,
        span_width: u32,
        span_height: u32,
        color: Color,
    ) {
        let (red, green, blue, alpha) = color.as_rgba_tuple();
        let source_alpha = u32::from(alpha);

        if source_alpha == 0 {
            return;
        }

        let source_red = u32::from(red) * source_alpha / 255;
        let source_green = u32::from(green) * source_alpha / 255;
        let source_blue = u32::from(blue) * source_alpha / 255;

        for row in 0..span_height {
            let py = self.origin.1 + y + row as i32;
            if py < 0 || py >= self.height as i32 {
                continue;
            }

            for column in 0..span_width {
                let px = self.origin.0 + x + column as i32;
                if px < 0 || px >= self.width as i32 {
                    continue;
                }

                let offset = (py as usize * self.width as usize + px as usize) * 4;
                let inverse = 255 - source_alpha;

                self.pixels[offset] =
                    (source_red + u32::from(self.pixels[offset]) * inverse / 255) as u8;
                self.pixels[offset + 1] =
                    (source_green + u32::from(self.pixels[offset + 1]) * inverse / 255) as u8;
                self.pixels[offset + 2] =
                    (source_blue + u32::from(self.pixels[offset + 2]) * inverse / 255) as u8;
                self.pixels[offset + 3] =
                    (source_alpha + u32::from(self.pixels[offset + 3]) * inverse / 255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_engine_accepts_a_small_surface() {
        let mut engine = TextEngine::new();
        let mut pixels = vec![0; 160 * 36 * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[20, 19, 19, 255]);
        }

        engine.draw_text(&mut pixels, 160, 36, "Rust shell", (8, 5), 14.0);

        assert_eq!(pixels.len(), 160 * 36 * 4);
    }
}
