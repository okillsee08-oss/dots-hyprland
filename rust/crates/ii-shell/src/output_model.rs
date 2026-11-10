//! Checked logical-to-buffer geometry for output-bound layer panels.
//!
//! Wayland layer configure sizes are surface-local (logical) dimensions.
//! With integer buffer scaling, the SHM buffer dimensions must be multiplied
//! by the output scale factor. Keeping this conversion pure makes overflow and
//! incomplete configure handling testable without a running compositor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelGeometry {
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale_factor: u32,
    pub buffer_width: u32,
    pub buffer_height: u32,
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum PanelGeometryError {
    #[error("layer configure omitted its width and the output has no logical width")]
    MissingLogicalWidth,
    #[error("panel width and height must be greater than zero")]
    ZeroDimension,
    #[error("output scale factor must be greater than zero")]
    InvalidScale,
    #[error("scaled SHM buffer dimensions overflowed")]
    BufferSizeOverflow,
}

impl PanelGeometry {
    pub fn from_configure(
        configured_size: (u32, u32),
        output_logical_width: Option<u32>,
        scale_factor: u32,
        fallback_height: u32,
    ) -> Result<Self, PanelGeometryError> {
        let logical_width = if configured_size.0 == 0 {
            output_logical_width
                .filter(|width| *width > 0)
                .ok_or(PanelGeometryError::MissingLogicalWidth)?
        } else {
            configured_size.0
        };
        let logical_height = if configured_size.1 == 0 {
            fallback_height
        } else {
            configured_size.1
        };

        Self::from_logical_size(logical_width, logical_height, scale_factor)
    }

    pub fn from_logical_size(
        logical_width: u32,
        logical_height: u32,
        scale_factor: u32,
    ) -> Result<Self, PanelGeometryError> {
        if scale_factor == 0 {
            return Err(PanelGeometryError::InvalidScale);
        }
        if logical_width == 0 || logical_height == 0 {
            return Err(PanelGeometryError::ZeroDimension);
        }

        let buffer_width = logical_width
            .checked_mul(scale_factor)
            .ok_or(PanelGeometryError::BufferSizeOverflow)?;
        let buffer_height = logical_height
            .checked_mul(scale_factor)
            .ok_or(PanelGeometryError::BufferSizeOverflow)?;

        Ok(Self {
            logical_width,
            logical_height,
            scale_factor,
            buffer_width,
            buffer_height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_one_keeps_logical_and_buffer_dimensions_equal() {
        assert_eq!(
            PanelGeometry::from_logical_size(1920, 36, 1).unwrap(),
            PanelGeometry {
                logical_width: 1920,
                logical_height: 36,
                scale_factor: 1,
                buffer_width: 1920,
                buffer_height: 36,
            }
        );
    }

    #[test]
    fn integer_scale_multiplies_both_buffer_dimensions() {
        let geometry = PanelGeometry::from_logical_size(2560, 36, 2).unwrap();
        assert_eq!((geometry.buffer_width, geometry.buffer_height), (5120, 72));
    }

    #[test]
    fn zero_configure_dimensions_use_output_and_panel_fallbacks() {
        let geometry = PanelGeometry::from_configure((0, 0), Some(3440), 1, 36).unwrap();
        assert_eq!(
            (geometry.logical_width, geometry.logical_height),
            (3440, 36)
        );
    }

    #[test]
    fn zero_width_without_output_fallback_is_rejected() {
        assert_eq!(
            PanelGeometry::from_configure((0, 36), None, 1, 36),
            Err(PanelGeometryError::MissingLogicalWidth)
        );
    }

    #[test]
    fn rejects_zero_scale_and_zero_dimensions() {
        assert_eq!(
            PanelGeometry::from_logical_size(1920, 36, 0),
            Err(PanelGeometryError::InvalidScale)
        );
        assert_eq!(
            PanelGeometry::from_logical_size(0, 36, 1),
            Err(PanelGeometryError::ZeroDimension)
        );
    }

    #[test]
    fn rejects_scaled_dimension_overflow() {
        assert_eq!(
            PanelGeometry::from_logical_size(u32::MAX, 36, 2),
            Err(PanelGeometryError::BufferSizeOverflow)
        );
    }
}
