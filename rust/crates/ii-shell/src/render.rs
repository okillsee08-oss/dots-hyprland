use tiny_skia::{Color, Paint, PixmapMut, Rect, Transform};

use crate::workspace_model::WorkspaceModel;

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

/// Draws the first native workspace strip for one output.
///
/// The sizing follows upstream Workspaces.qml (26 logical px per slot and a
/// 22 px active indicator). Application icons remain a follow-up; active
/// special workspaces receive a compact label pill after the normal slots.
pub fn draw_workspace_strip(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    model: &WorkspaceModel,
    scale: u32,
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

    // In the default upstream config, app icons are shown and workspace
    // numbers are hidden; their compact dot indicators remain visible.
    let dot = 5.0 * scale;
    for (index, slot) in model.slots.iter().enumerate() {
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
    use crate::workspace_model::WorkspaceSlot;

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
            special_workspace_name: Some("chat".into()),
        };

        draw_workspace_strip(&mut pixels, width, height, &model, 1);

        let offset = ((7 * width + 86) * 4) as usize;
        assert_eq!(&pixels[offset..offset + 4], &[178, 83, 93, 255]);
        assert_eq!(special_workspace_width("chat", 1), 46);
        assert_eq!(special_workspace_width("a very long special workspace", 1), 142);
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
            special_workspace_name: None,
        };

        draw_workspace_strip(&mut pixels, width, height, &model, 1);
        draw_workspace_strip(&mut pixels, width, height, &model, 0);
        assert_eq!(pixels, original);
    }
}
