//! Map a sensor's own axes to the native display before hit testing.

/// Normalize independently before rotating: on a landscape panel the
/// sensor may still report portrait axes, with a different unit per pixel.
/// The display dimensions are applied only after that axis transformation.
pub(crate) fn map(raw: (i32, i32), x_range: (i32, i32), y_range: (i32, i32), w: f32, h: f32, rotation: u16) -> (f32, f32) {
    let span = |r: (i32, i32)| (r.1 - r.0).max(1) as f32;
    let x = (raw.0 - x_range.0) as f32 / span(x_range);
    let y = (raw.1 - y_range.0) as f32 / span(y_range);
    let (x, y) = match rotation {
        0 => (x, y),
        90 => (1.0 - y, x),
        180 => (1.0 - x, 1.0 - y),
        270 => (y, 1.0 - x),
        _ => unreachable!("touch rotation is validated when the device config is read"),
    };
    (x * w, y * h)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAD_X: (i32, i32) = (0, 23999);
    const PAD_Y: (i32, i32) = (0, 33919);
    const NATIVE: (f32, f32) = (3392.0, 2400.0);

    fn pad(raw: (i32, i32), rotation: u16) -> (f32, f32) {
        map(raw, PAD_X, PAD_Y, NATIVE.0, NATIVE.1, rotation)
    }

    #[test]
    fn portrait_sensor_corners_map_to_landscape_display_for_either_direction() {
        let corners = [(0, 0), (23999, 0), (23999, 33919), (0, 33919)];
        let clockwise = [(3392.0, 0.0), (3392.0, 2400.0), (0.0, 2400.0), (0.0, 0.0)];
        let counterclockwise = [(0.0, 2400.0), (0.0, 0.0), (3392.0, 0.0), (3392.0, 2400.0)];
        for (i, raw) in corners.into_iter().enumerate() {
            assert_eq!(pad(raw, 90), clockwise[i]);
            assert_eq!(pad(raw, 270), counterclockwise[i]);
        }
    }

    #[test]
    fn nearest_sensor_center_stays_at_the_display_center() {
        // Both advertised maxima are odd, so no integer sample is exactly
        // at half the sensor range. The nearest sample is within a tenth
        // of a native pixel for every supported rotation.
        for rotation in [0, 90, 180, 270] {
            let (x, y) = pad((12000, 16960), rotation);
            assert!((x - NATIVE.0 / 2.0).abs() < 0.1, "{rotation}: {x}");
            assert!((y - NATIVE.1 / 2.0).abs() < 0.1, "{rotation}: {y}");
        }
    }

    #[test]
    fn unrotated_devices_keep_their_axis_scaling_and_offsets() {
        assert_eq!(map((110, 220), (10, 210), (20, 420), 1440.0, 3200.0, 0), (720.0, 1600.0));
        assert_eq!(pad((23999, 33919), 0), NATIVE);
        assert_eq!(pad((23999, 33919), 180), (0.0, 0.0));
        assert_eq!(map((5, 7), (5, 5), (7, 7), 10.0, 20.0, 0), (0.0, 0.0));
    }

    #[test]
    fn sensor_axis_motion_follows_the_rotated_display_axis() {
        let left = pad((0, 16960), 90);
        let right = pad((23999, 16960), 90);
        assert_eq!(left.0, right.0);
        assert_eq!((left.1, right.1), (0.0, 2400.0));
        let top = pad((12000, 0), 270);
        let bottom = pad((12000, 33919), 270);
        assert_eq!(top.1, bottom.1);
        assert_eq!((top.0, bottom.0), (0.0, 3392.0));
    }
}
