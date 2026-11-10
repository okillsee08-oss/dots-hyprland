//! Allocation-free redraw state for one Wayland surface.
//!
//! Redraw requests coalesce while a frame callback is outstanding. A request
//! remains pending if no SHM slot is writable and can be retried after the
//! main Wayland queue dispatches a buffer-release event.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameScheduler {
    frame_pending: bool,
    redraw_pending: bool,
}

impl FrameScheduler {
    pub fn request_redraw(&mut self) {
        self.redraw_pending = true;
    }

    pub fn can_draw(&self) -> bool {
        self.redraw_pending && !self.frame_pending
    }

    /// Call after a buffer attach and surface commit have been queued.
    pub fn frame_submitted(&mut self) {
        self.frame_pending = true;
        self.redraw_pending = false;
    }

    /// Complete the current frame callback and report whether a new invalidation
    /// arrived while the previous frame was in flight.
    pub fn frame_completed(&mut self) -> bool {
        self.frame_pending = false;
        self.can_draw()
    }

    pub fn is_frame_pending(&self) -> bool {
        self.frame_pending
    }

    pub fn is_redraw_pending(&self) -> bool {
        self.redraw_pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_redraws_during_an_in_flight_frame() {
        let mut scheduler = FrameScheduler::default();
        scheduler.request_redraw();
        assert!(scheduler.can_draw());

        scheduler.frame_submitted();
        assert!(scheduler.is_frame_pending());
        scheduler.request_redraw();
        scheduler.request_redraw();
        scheduler.request_redraw();

        assert!(scheduler.is_redraw_pending());
        assert!(scheduler.frame_completed());
        assert!(scheduler.can_draw());
        assert!(!scheduler.is_frame_pending());
    }

    #[test]
    fn redraw_remains_pending_until_a_buffer_can_be_submitted() {
        let mut scheduler = FrameScheduler::default();
        scheduler.request_redraw();
        assert!(scheduler.can_draw());
        assert!(scheduler.is_redraw_pending());

        // A failed buffer lookup does not call frame_submitted().
        scheduler.frame_submitted();
        assert!(!scheduler.can_draw());
        assert!(!scheduler.is_redraw_pending());
    }

    #[test]
    fn frame_completion_without_new_invalidation_does_not_repaint() {
        let mut scheduler = FrameScheduler::default();
        scheduler.request_redraw();
        scheduler.frame_submitted();

        assert!(!scheduler.frame_completed());
        assert!(!scheduler.can_draw());
    }
}
