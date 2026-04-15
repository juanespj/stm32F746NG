use embedded_graphics::prelude::Point;
use kolibri_embedded_gui::ui::Interaction;

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
