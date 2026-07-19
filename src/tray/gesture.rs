use std::time::{Duration, Instant};

use windows_sys::Win32::UI::WindowsAndMessaging::{
    WHEEL_DELTA, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

const ACCELERATION_RESET: Duration = Duration::from_millis(350);

#[derive(Clone, Copy)]
pub(crate) struct WheelEvent {
    pub(crate) steps: i32,
    pub(crate) timestamp: Instant,
    pub(crate) generation: u32,
}

#[derive(Default)]
pub(crate) struct WheelAccelerator {
    direction: i32,
    streak: u32,
    last_event: Option<Instant>,
}

impl WheelAccelerator {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn adjust(&mut self, event: WheelEvent) -> i32 {
        let direction = event.steps.signum();
        let starts_new_streak = direction != self.direction
            || self.last_event.is_none_or(|last| {
                event.timestamp.saturating_duration_since(last) > ACCELERATION_RESET
            });
        if starts_new_streak {
            self.streak = 0;
            self.direction = direction;
        }
        self.last_event = Some(event.timestamp);

        (0..event.steps.unsigned_abs()).fold(0_i32, |adjustment, _| {
            self.streak = self.streak.saturating_add(1);
            adjustment.saturating_add(direction.saturating_mul(step_size(self.streak)))
        })
    }
}

pub(crate) fn steps_from_mouse_data(mouse_data: u32) -> i32 {
    let high_word = (mouse_data >> 16) as u16;
    let delta = i32::from(i16::from_ne_bytes(high_word.to_ne_bytes()));
    let wheel_delta = i32::try_from(WHEEL_DELTA).expect("WHEEL_DELTA fits in i32");
    if delta.abs() >= wheel_delta {
        delta / wheel_delta
    } else {
        delta.signum()
    }
}

pub(crate) fn is_double_click(previous: u32, current: u32, maximum_interval: u32) -> bool {
    previous != 0 && current.wrapping_sub(previous) <= maximum_interval
}

pub(crate) fn needs_tray_hit_test(
    message: u32,
    interaction_active: bool,
    click_pending: bool,
) -> bool {
    match message {
        WM_MOUSEMOVE => interaction_active || click_pending,
        WM_MOUSEWHEEL | WM_LBUTTONUP => true,
        _ => false,
    }
}

fn step_size(streak: u32) -> i32 {
    match streak {
        1..=2 => 2,
        3..=5 => 4,
        6..=9 => 7,
        _ => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WheelAccelerator, WheelEvent, is_double_click, needs_tray_hit_test, step_size,
        steps_from_mouse_data,
    };
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP,
    };

    fn event(steps: i32, timestamp: Instant) -> WheelEvent {
        WheelEvent {
            steps,
            timestamp,
            generation: 0,
        }
    }

    fn mouse_data(delta: i16) -> u32 {
        u32::from(u16::from_ne_bytes(delta.to_ne_bytes())) << 16
    }

    #[test]
    fn acceleration_curve_has_explicit_boundaries() {
        let values = (1..=11).map(step_size).collect::<Vec<_>>();
        assert_eq!(values, [2, 2, 4, 4, 4, 7, 7, 7, 7, 10, 10]);
    }

    #[test]
    fn accelerates_a_continuous_burst() {
        let start = Instant::now();
        let mut accelerator = WheelAccelerator::default();
        let adjustments = (0..10)
            .map(|index| accelerator.adjust(event(1, start + Duration::from_millis(index * 20))))
            .collect::<Vec<_>>();

        assert_eq!(adjustments, [2, 2, 4, 4, 4, 7, 7, 7, 7, 10]);
    }

    #[test]
    fn applies_each_notch_in_a_multi_step_event() {
        let mut accelerator = WheelAccelerator::default();
        assert_eq!(accelerator.adjust(event(3, Instant::now())), 8);
    }

    #[test]
    fn resets_after_pause_or_direction_change() {
        let start = Instant::now();
        let mut accelerator = WheelAccelerator::default();
        assert_eq!(accelerator.adjust(event(3, start)), 8);
        assert_eq!(
            accelerator.adjust(event(1, start + Duration::from_millis(500))),
            2
        );
        assert_eq!(
            accelerator.adjust(event(-1, start + Duration::from_millis(520))),
            -2
        );
    }

    #[test]
    fn converts_standard_and_high_resolution_wheel_deltas() {
        assert_eq!(steps_from_mouse_data(mouse_data(120)), 1);
        assert_eq!(steps_from_mouse_data(mouse_data(-120)), -1);
        assert_eq!(steps_from_mouse_data(mouse_data(240)), 2);
        assert_eq!(steps_from_mouse_data(mouse_data(30)), 1);
        assert_eq!(steps_from_mouse_data(mouse_data(-30)), -1);
    }

    #[test]
    fn recognizes_double_clicks_across_timestamp_wraparound() {
        assert!(!is_double_click(0, 100, 500));
        assert!(is_double_click(100, 450, 500));
        assert!(!is_double_click(100, 601, 500));
        assert!(is_double_click(u32::MAX - 10, 5, 20));
    }

    #[test]
    fn avoids_shell_hit_tests_for_unrelated_and_idle_mouse_events() {
        assert!(!needs_tray_hit_test(WM_MOUSEMOVE, false, false));
        assert!(needs_tray_hit_test(WM_MOUSEMOVE, true, false));
        assert!(needs_tray_hit_test(WM_MOUSEMOVE, false, true));
        assert!(needs_tray_hit_test(WM_MOUSEWHEEL, false, false));
        assert!(needs_tray_hit_test(WM_LBUTTONUP, false, false));
        assert!(!needs_tray_hit_test(WM_RBUTTONUP, true, true));
    }
}
