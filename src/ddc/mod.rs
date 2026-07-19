mod windows;

use crate::{DdcError, Result};

use self::windows::PhysicalMonitor;

/// VCP feature code defined by MCCS for display luminance.
pub const BRIGHTNESS_VCP_CODE: u8 = 0x10;

/// Raw brightness values reported by a physical monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Brightness {
    /// Current monitor brightness value.
    pub current: u32,
    /// Maximum monitor brightness value.
    pub maximum: u32,
}

impl Brightness {
    /// Returns the brightness rounded to the nearest percentage point.
    #[must_use]
    pub fn percent(self) -> u32 {
        value_to_percent(self.current, self.maximum)
    }
}

/// Information about one physical monitor discovered through Windows.
#[derive(Debug)]
pub struct MonitorInfo {
    /// Stable index for this enumeration, with primary monitors first.
    pub index: usize,
    /// Monitor description reported by the Windows physical monitor API.
    pub description: String,
    /// Whether this monitor belongs to the primary logical display.
    pub primary: bool,
    /// Brightness, or the DDC/CI error returned by this monitor.
    pub brightness: Result<Brightness>,
}

/// Lists physical monitors, placing monitors on the primary display first.
///
/// # Errors
///
/// Returns an error when Windows cannot enumerate any physical monitors.
pub fn list() -> Result<Vec<MonitorInfo>> {
    let monitors = windows::enumerate()?;
    Ok(monitors
        .iter()
        .enumerate()
        .map(|(index, monitor)| MonitorInfo {
            index,
            description: monitor.description().to_owned(),
            primary: monitor.is_primary(),
            brightness: monitor.read_brightness(),
        })
        .collect())
}

/// Reads brightness from the physical monitor at `index`.
///
/// # Errors
///
/// Returns an error when `index` does not exist or the monitor does not answer
/// the DDC/CI brightness query.
pub fn get(index: usize) -> Result<Brightness> {
    let monitors = windows::enumerate()?;
    select(&monitors, index)?.read_brightness()
}

/// Sets the physical monitor at `index` to `percent` brightness.
///
/// # Errors
///
/// Returns an error when `percent` is greater than 100, `index` does not exist,
/// or the monitor rejects the DDC/CI operation.
pub fn set(index: usize, percent: u32) -> Result<Brightness> {
    validate_percent(percent)?;
    let monitors = windows::enumerate()?;
    select(&monitors, index)?.set_brightness(percent)
}

/// Changes brightness on the physical monitor at `index` by `delta` percent.
///
/// # Errors
///
/// Returns an error when `index` does not exist or the monitor rejects a DDC/CI
/// read or write. The resulting percentage is clamped to `0..=100`.
pub fn change(index: usize, delta: i32) -> Result<Brightness> {
    let monitors = windows::enumerate()?;
    let monitor = select(&monitors, index)?;
    let current = monitor.read_brightness()?.percent() as i32;
    let target = (current + delta).clamp(0, 100) as u32;
    monitor.set_brightness(target)
}

/// Returns the raw MCCS capabilities string for the physical monitor at `index`.
///
/// # Errors
///
/// Returns an error when `index` does not exist or the monitor does not provide
/// a capabilities string.
pub fn capabilities(index: usize) -> Result<String> {
    let monitors = windows::enumerate()?;
    select(&monitors, index)?.capabilities()
}

fn select(monitors: &[PhysicalMonitor], index: usize) -> Result<&PhysicalMonitor> {
    monitors.get(index).ok_or_else(|| {
        DdcError::message(format!(
            "monitor index {index} is out of range; found {} physical monitor(s)",
            monitors.len()
        ))
    })
}

fn validate_percent(percent: u32) -> Result<()> {
    if percent <= 100 {
        Ok(())
    } else {
        Err(DdcError::message(format!(
            "brightness must be between 0 and 100, got {percent}"
        )))
    }
}

fn value_to_percent(value: u32, maximum: u32) -> u32 {
    if maximum == 0 {
        return 0;
    }
    ((u64::from(value) * 100 + u64::from(maximum) / 2) / u64::from(maximum)) as u32
}

pub(super) fn percent_to_value(percent: u32, maximum: u32) -> u32 {
    ((u64::from(percent) * u64::from(maximum) + 50) / 100) as u32
}

#[cfg(test)]
mod tests {
    use super::{Brightness, percent_to_value, validate_percent, value_to_percent};

    #[test]
    fn converts_non_hundred_ranges() {
        assert_eq!(value_to_percent(127, 255), 50);
        assert_eq!(percent_to_value(50, 255), 128);
    }

    #[test]
    fn preserves_endpoints_for_common_monitor_ranges() {
        for maximum in [1, 100, 255, 65_535] {
            assert_eq!(value_to_percent(0, maximum), 0);
            assert_eq!(value_to_percent(maximum, maximum), 100);
            assert_eq!(percent_to_value(0, maximum), 0);
            assert_eq!(percent_to_value(100, maximum), maximum);
        }
    }

    #[test]
    fn round_trip_stays_within_one_percentage_point() {
        for maximum in [100, 255, 1_000, 65_535] {
            for percent in 0..=100 {
                let round_trip = value_to_percent(percent_to_value(percent, maximum), maximum);
                assert!(round_trip.abs_diff(percent) <= 1);
            }
        }
    }

    #[test]
    fn zero_maximum_converts_to_zero() {
        assert_eq!(
            Brightness {
                current: 25,
                maximum: 0
            }
            .percent(),
            0
        );
    }

    #[test]
    fn validates_percent_before_touching_hardware() {
        assert!(validate_percent(0).is_ok());
        assert!(validate_percent(100).is_ok());
        assert_eq!(
            validate_percent(101)
                .expect_err("101 must be rejected")
                .to_string(),
            "brightness must be between 0 and 100, got 101"
        );
    }
}
