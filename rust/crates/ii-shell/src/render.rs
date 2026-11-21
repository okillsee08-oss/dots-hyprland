use tiny_skia::{Color, Paint, Pixmap, PixmapMut, Rect, Transform};

use crate::{battery::BatterySnapshot, icon_lookup::IconResolver, workspace_model::WorkspaceModel};

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

/// Logical size of the compact upstream-style clipped battery progress bar.
pub const BATTERY_INDICATOR_WIDTH: u32 = 56;
pub const BATTERY_INDICATOR_HEIGHT: u32 = 22;

/// Draw the upstream battery indicator into the existing panel SHM buffer.
///
/// The track and fill are rasterized directly into the caller-owned buffer.
/// No temporary pixmap, path allocation, image resource, or timer is created.
/// The caller overlays the percentage text using the shared TextEngine.
pub fn draw_battery_indicator(
    pixels: &mut [u8],
    buffer_width: u32,
    buffer_height: u32,
    x: i32,
    y: i32,
    scale: u32,
    snapshot: BatterySnapshot,
) {
    if scale == 0 || !snapshot.available || buffer_width == 0 || buffer_height == 0 {
        return;
    }

    let Some(required_len) = (buffer_width as usize)
        .checked_mul(buffer_height as usize)
        .and_then(|size| size.checked_mul(4))
    else {
        return;
    };
    if pixels.len() < required_len {
        return;
    }

    let Ok(scale) = i32::try_from(scale) else {
        return;
    };
    let width = (BATTERY_INDICATOR_WIDTH as i32).saturating_mul(scale);
    let height = (BATTERY_INDICATOR_HEIGHT as i32).saturating_mul(scale);
    if width <= 0 || height <= 0 {
        return;
    }

    fill_rounded_rect_pixels(
        pixels,
        buffer_width,
        buffer_height,
        x,
        y,
        width,
        height,
        11 * scale,
        [73, 70, 74, 255],
    );

    let percentage = if snapshot.percentage.is_finite() {
        snapshot.percentage.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fill_width = (f64::from(width) * percentage).round() as i32;
    if fill_width > 0 {
        let fill_color = if snapshot.is_low && !snapshot.is_charging {
            [255, 180, 171, 255]
        } else {
            [236, 230, 233, 255]
        };
        fill_rounded_rect_pixels(
            pixels,
            buffer_width,
            buffer_height,
            x,
            y,
            fill_width,
            height,
            2 * scale,
            fill_color,
        );
    }

    // Material-style bolt glyph, drawn only while charging and below 100%.
    // A small fixed polygon avoids requiring an icon font or another cache.
    if snapshot.is_charging && percentage < 1.0 {
        let points = [
            (x + 11 * scale, y + 4 * scale),
            (x + 7 * scale, y + 12 * scale),
            (x + 11 * scale, y + 12 * scale),
            (x + 9 * scale, y + 18 * scale),
            (x + 16 * scale, y + 9 * scale),
            (x + 12 * scale, y + 9 * scale),
            (x + 14 * scale, y + 4 * scale),
        ];
        fill_polygon_pixels(
            pixels,
            buffer_width,
            buffer_height,
            &points,
            [49, 48, 48, 255],
        );
    }
}

fn fill_rounded_rect_pixels(
    pixels: &mut [u8],
    buffer_width: u32,
    buffer_height: u32,
    x: i32,
    y: i32,
    rect_width: i32,
    rect_height: i32,
    radius: i32,
    color: [u8; 4],
) {
    if rect_width <= 0 || rect_height <= 0 {
        return;
    }

    let right = x.saturating_add(rect_width).min(buffer_width as i32);
    let bottom = y.saturating_add(rect_height).min(buffer_height as i32);
    let left = x.max(0);
    let top = y.max(0);
    if left >= right || top >= bottom {
        return;
    }

    let radius = radius.max(0).min(rect_width / 2).min(rect_height / 2);
    let radius_squared = i64::from(radius) * i64::from(radius);
    for py in top..bottom {
        let nearest_y = if py < y + radius {
            y + radius
        } else if py >= y + rect_height - radius {
            y + rect_height - radius - 1
        } else {
            py
        };
        for px in left..right {
            let nearest_x = if px < x + radius {
                x + radius
            } else if px >= x + rect_width - radius {
                x + rect_width - radius - 1
            } else {
                px
            };
            let dx = i64::from(px - nearest_x);
            let dy = i64::from(py - nearest_y);
            if radius == 0 || dx * dx + dy * dy <= radius_squared {
                let offset = ((py as usize * buffer_width as usize) + px as usize) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
}

fn fill_polygon_pixels(
    pixels: &mut [u8],
    buffer_width: u32,
    buffer_height: u32,
    points: &[(i32, i32)],
    color: [u8; 4],
) {
    if points.len() < 3 {
        return;
    }

    let min_x = points.iter().map(|point| point.0).min().unwrap_or(0).max(0);
    let max_x = points
        .iter()
        .map(|point| point.0)
        .max()
        .unwrap_or(0)
        .min(buffer_width as i32);
    let min_y = points.iter().map(|point| point.1).min().unwrap_or(0).max(0);
    let max_y = points
        .iter()
        .map(|point| point.1)
        .max()
        .unwrap_or(0)
        .min(buffer_height as i32);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    for py in min_y..max_y {
        for px in min_x..max_x {
            if point_in_polygon(px as f32 + 0.5, py as f32 + 0.5, points) {
                let offset = ((py as usize * buffer_width as usize) + px as usize) * 4;
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
}

fn point_in_polygon(x: f32, y: f32, points: &[(i32, i32)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (current_x, current_y) = points[current];
        let (previous_x, previous_y) = points[previous];
        let current_y = current_y as f32;
        let previous_y = previous_y as f32;
        if (current_y > y) != (previous_y > y) {
            let intersection_x = (previous_x - current_x) as f32 * (y - current_y)
                / (previous_y - current_y)
                + current_x as f32;
            if x < intersection_x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

/// Draws the native workspace strip without an icon resolver. This wrapper
/// keeps simple renderer callers and tests independent of host icon themes.
pub fn draw_workspace_strip(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    model: &WorkspaceModel,
    scale: u32,
) {
    draw_workspace_strip_with_icons(pixels, width, height, model, scale, None, true);
}

/// Draws the workspace strip and, when available, small PNG app icons.
/// SVG/XPM names are still resolved by IconResolver but fall back to the
/// occupancy dot until a low-overhead vector rasterizer is selected.
pub fn draw_workspace_strip_with_icons(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    model: &WorkspaceModel,
    scale: u32,
    mut icons: Option<&mut IconResolver>,
    show_app_icons: bool,
) {
    if scale == 0 || model.slots.is_empty() {
        return;
    }
    let Some(mut pixmap) = PixmapMut::from_bytes(pixels, width, height) else {
        return;
    };

    const BUTTON: f32 = 26.0;
    const ACTIVE: f32 = 22.0;
    let scale = scale as f32;
    let button = BUTTON * scale;
    let active = ACTIVE * scale;
    let origin_x = 8.0 * scale;
    let center_y = height as f32 / 2.0;
    let top = center_y - active / 2.0;

    // Occupied workspaces join into a continuous pill, as in the upstream
    // WorkspaceModel/Workspaces components. Fake empty active workspaces are
    // deliberately excluded by WorkspaceSlot::visibly_occupied().
    for (index, slot) in model.slots.iter().enumerate() {
        if !slot.visibly_occupied() {
            continue;
        }
        let previous = index > 0 && model.slots[index - 1].visibly_occupied();
        let next = index + 1 < model.slots.len() && model.slots[index + 1].visibly_occupied();
        let left_extension = if previous { button / 2.0 } else { 0.0 };
        let right_extension = if next { button / 2.0 } else { 0.0 };
        let x = origin_x + index as f32 * button - left_extension + (button - active) / 2.0;
        let pill_width = active + left_extension + right_extension;
        fill_rect(
            &mut pixmap,
            x,
            top,
            pill_width,
            active,
            Color::from_rgba8(67, 63, 64, 255),
        );
    }

    // The upstream active indicator is a 22 px pill centered in its 26 px slot.
    if let Some(active_x) = model
        .active_index
        .checked_mul(BUTTON as usize)
        .map(|offset| origin_x + offset as f32 * scale + (button - active) / 2.0)
        .filter(|_| model.active_index < model.slots.len())
    {
        fill_rect(
            &mut pixmap,
            active_x,
            top,
            active,
            active,
            Color::from_rgba8(178, 83, 93, 255),
        );
    }

    // The upstream default enables app icons. Use a bounded cached PNG when
    // one is available; otherwise preserve the occupied/empty dot indicator.
    let dot = 5.0 * scale;
    for (index, slot) in model.slots.iter().enumerate() {
        let icon = if show_app_icons {
            model.biggest_window_at(index).and_then(|client| {
                icons
                    .as_deref_mut()
                    .and_then(|resolver| resolver.png_for_class(&client.app_class))
            })
        } else {
            None
        };
        if let Some(icon) = icon {
            let icon_size = (18.0 * scale).round().max(1.0) as u32;
            let x = (origin_x + index as f32 * button + (button - icon_size as f32) / 2.0).round()
                as u32;
            let y = (center_y - icon_size as f32 / 2.0).round() as u32;
            draw_icon(&mut pixmap, icon, x, y, icon_size, width, height);
        } else {
            let x = origin_x + index as f32 * button + (button - dot) / 2.0;
            let y = center_y - dot / 2.0;
            let color = if index == model.active_index {
                Color::from_rgba8(255, 245, 245, 255)
            } else if slot.visibly_occupied() {
                Color::from_rgba8(224, 219, 220, 255)
            } else {
                Color::from_rgba8(125, 119, 120, 255)
            };
            fill_rect(&mut pixmap, x, y, dot, dot, color);
        }
    }

    if let Some(name) = model.special_workspace_name.as_deref() {
        let label_width = special_workspace_width(name, scale as u32) as f32;
        fill_rect(
            &mut pixmap,
            origin_x + model.slots.len() as f32 * button,
            top,
            label_width,
            active,
            Color::from_rgba8(178, 83, 93, 255),
        );
    }
}

/// Draw a cached premultiplied RGBA icon into the caller-owned panel buffer.
/// Nearest-neighbor scaling keeps this tiny 18 px marker allocation-free.
fn draw_icon(
    destination: &mut PixmapMut<'_>,
    source: &Pixmap,
    x: u32,
    y: u32,
    size: u32,
    destination_width: u32,
    destination_height: u32,
) {
    if size == 0 || source.width() == 0 || source.height() == 0 {
        return;
    }

    let fit = size as f32 / source.width().max(source.height()) as f32;
    let draw_width = (source.width() as f32 * fit).round().max(1.0) as u32;
    let draw_height = (source.height() as f32 * fit).round().max(1.0) as u32;
    let left = x + (size.saturating_sub(draw_width)) / 2;
    let top = y + (size.saturating_sub(draw_height)) / 2;
    let draw_width = draw_width.min(destination_width.saturating_sub(left));
    let draw_height = draw_height.min(destination_height.saturating_sub(top));
    if draw_width == 0 || draw_height == 0 {
        return;
    }

    let source_pixels = source.data();
    let destination_pixels = destination.data_mut();
    for target_y in 0..draw_height {
        let source_y = (target_y as u64 * source.height() as u64 / draw_height as u64) as u32;
        for target_x in 0..draw_width {
            let source_x = (target_x as u64 * source.width() as u64 / draw_width as u64) as u32;
            let source_offset = ((source_y * source.width() + source_x) * 4) as usize;
            let target_offset =
                (((top + target_y) * destination_width + left + target_x) * 4) as usize;
            let alpha = source_pixels[source_offset + 3] as u32;
            let inverse_alpha = 255 - alpha;

            for channel in 0..3 {
                let source_channel = source_pixels[source_offset + channel] as u32;
                let target_channel = destination_pixels[target_offset + channel] as u32;
                destination_pixels[target_offset + channel] = source_channel
                    .saturating_add((target_channel * inverse_alpha + 127) / 255)
                    .min(255) as u8;
            }
            let target_alpha = destination_pixels[target_offset + 3] as u32;
            destination_pixels[target_offset + 3] = alpha
                .saturating_add((target_alpha * inverse_alpha + 127) / 255)
                .min(255) as u8;
        }
    }
}

/// Approximate label-pill width shared by the raster background and text pass.
/// Cap the label so a long special-workspace name cannot consume the full bar.
pub fn special_workspace_width(name: &str, scale: u32) -> u32 {
    (22 + name.chars().count().min(20) as u32 * 6) * scale
}

fn fill_rect(pixmap: &mut PixmapMut<'_>, x: f32, y: f32, width: f32, height: f32, color: Color) {
    let Some(rect) = Rect::from_xywh(x, y, width, height) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color(color);
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        battery::{BatteryThresholds, DeviceState, UPowerBatteryProperties},
        workspace_model::WorkspaceSlot,
    };

    fn battery_snapshot(percent: f64, state: DeviceState) -> BatterySnapshot {
        BatterySnapshot::from_upower(
            UPowerBatteryProperties {
                device_type: 2,
                power_supply: true,
                state: state as u32,
                percentage_percent: percent,
                energy_rate_w: 12.0,
                time_to_empty_seconds: 3_600,
                time_to_full_seconds: 1_800,
                energy_full: Some(45.0),
                energy_full_design: Some(50.0),
            },
            BatteryThresholds::default(),
        )
    }

    #[test]
    fn battery_indicator_draws_proportional_fill_and_track() {
        let width = 100u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);

        draw_battery_indicator(
            &mut pixels,
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(50.0, DeviceState::Discharging),
        );

        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(15, 18), [236, 230, 233, 255]);
        assert_eq!(pixel(55, 18), [73, 70, 74, 255]);
    }

    #[test]
    fn low_battery_uses_error_fill_color() {
        let width = 100u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);

        draw_battery_indicator(
            &mut pixels,
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(4.0, DeviceState::Discharging),
        );

        let offset = ((18 * width + 11) * 4) as usize;
        assert_eq!(&pixels[offset..offset + 4], &[255, 180, 171, 255]);
    }

    #[test]
    fn charging_bolt_is_drawn_only_while_charging_below_full() {
        let width = 100u32;
        let height = 36u32;
        let mut charging_pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut charging_pixels, width, height);
        draw_battery_indicator(
            &mut charging_pixels,
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(50.0, DeviceState::Charging),
        );
        let offset = ((12 * width + 22) * 4) as usize;
        assert_eq!(&charging_pixels[offset..offset + 4], &[49, 48, 48, 255]);

        let mut discharging_pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut discharging_pixels, width, height);
        draw_battery_indicator(
            &mut discharging_pixels,
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(50.0, DeviceState::Discharging),
        );
        assert_eq!(
            &discharging_pixels[offset..offset + 4],
            &[236, 230, 233, 255]
        );

        let mut full_pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut full_pixels, width, height);
        draw_battery_indicator(
            &mut full_pixels,
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(100.0, DeviceState::Charging),
        );
        assert_eq!(&full_pixels[offset..offset + 4], &[236, 230, 233, 255]);
    }

    #[test]
    fn battery_indicator_handles_zero_scale_invalid_buffers_and_missing_devices() {
        let width = 100u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);
        let original = pixels.clone();

        draw_battery_indicator(
            &mut pixels,
            width,
            height,
            10,
            7,
            0,
            battery_snapshot(50.0, DeviceState::Discharging),
        );
        let short_len = pixels.len() - 1;
        draw_battery_indicator(
            &mut pixels[..short_len],
            width,
            height,
            10,
            7,
            1,
            battery_snapshot(50.0, DeviceState::Discharging),
        );
        let mut unavailable = battery_snapshot(50.0, DeviceState::Discharging);
        unavailable.available = false;
        draw_battery_indicator(&mut pixels, width, height, 10, 7, 1, unavailable);

        assert_eq!(pixels, original);
    }

    #[test]
    fn fills_the_supplied_buffer_without_changing_its_size() {
        let width = 20;
        let height = 8;
        let mut pixels = vec![0; width * height * 4];
        draw_panel(&mut pixels, width as u32, height as u32);

        assert_eq!(pixels.len(), width * height * 4);
        assert_eq!(&pixels[0..4], &[178, 83, 93, 255]);
        assert_eq!(&pixels[20..24], &[20, 19, 19, 255]);
    }

    #[test]
    fn draws_active_and_occupied_slots_into_the_existing_buffer() {
        let width = 360u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);
        let model = WorkspaceModel {
            monitor_name: "eDP-1".into(),
            active_workspace_id: 1,
            group: 0,
            active_index: 0,
            fake_workspace_id: -9999,
            slots: vec![
                WorkspaceSlot {
                    id: 1,
                    occupied: false,
                    is_fake: false,
                },
                WorkspaceSlot {
                    id: 2,
                    occupied: false,
                    is_fake: false,
                },
                WorkspaceSlot {
                    id: 3,
                    occupied: true,
                    is_fake: false,
                },
            ],
            biggest_windows: vec![None; 3],
            special_workspace_name: None,
        };

        draw_workspace_strip(&mut pixels, width, height, &model, 1);

        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let offset = ((y * width + x) * 4) as usize;
            pixels[offset..offset + 4].try_into().unwrap()
        };
        assert_eq!(pixel(10, 7), [178, 83, 93, 255]);
        assert_eq!(pixel(63, 8), [67, 63, 64, 255]);
        assert_eq!(pixel(73, 18), [224, 219, 220, 255]);
    }

    #[test]
    fn draws_a_label_pill_for_an_active_special_workspace() {
        let width = 360u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);
        let model = WorkspaceModel {
            monitor_name: "eDP-1".into(),
            active_workspace_id: 1,
            group: 0,
            active_index: 0,
            fake_workspace_id: -9999,
            slots: vec![
                WorkspaceSlot {
                    id: 1,
                    occupied: false,
                    is_fake: false,
                },
                WorkspaceSlot {
                    id: 2,
                    occupied: false,
                    is_fake: false,
                },
                WorkspaceSlot {
                    id: 3,
                    occupied: false,
                    is_fake: false,
                },
            ],
            biggest_windows: vec![None; 3],
            special_workspace_name: Some("chat".into()),
        };

        draw_workspace_strip(&mut pixels, width, height, &model, 1);

        let offset = ((7 * width + 86) * 4) as usize;
        assert_eq!(&pixels[offset..offset + 4], &[178, 83, 93, 255]);
        assert_eq!(special_workspace_width("chat", 1), 46);
        assert_eq!(
            special_workspace_width("a very long special workspace", 1),
            142
        );
    }

    #[test]
    fn draws_a_cached_icon_into_the_existing_workspace_buffer() {
        let width = 40u32;
        let height = 36u32;
        let mut source = Pixmap::new(2, 2).unwrap();
        source.fill(Color::from_rgba8(220, 40, 50, 255));
        let mut pixels = vec![0; (width * height * 4) as usize];
        {
            let mut destination = PixmapMut::from_bytes(&mut pixels, width, height).unwrap();
            draw_icon(&mut destination, &source, 11, 9, 18, width, height);
        }

        let offset = ((18 * width + 20) * 4) as usize;
        assert_eq!(&pixels[offset..offset + 4], &[220, 40, 50, 255]);
    }

    #[test]
    fn skips_workspace_strip_for_zero_scale_or_empty_models() {
        let width = 64u32;
        let height = 36u32;
        let mut pixels = vec![0; (width * height * 4) as usize];
        draw_panel(&mut pixels, width, height);
        let original = pixels.clone();
        let model = WorkspaceModel {
            monitor_name: "eDP-1".into(),
            active_workspace_id: 1,
            group: 0,
            active_index: 0,
            fake_workspace_id: -9999,
            slots: Vec::new(),
            biggest_windows: Vec::new(),
            special_workspace_name: None,
        };

        draw_workspace_strip(&mut pixels, width, height, &model, 1);
        draw_workspace_strip(&mut pixels, width, height, &model, 0);
        assert_eq!(pixels, original);
    }
}
