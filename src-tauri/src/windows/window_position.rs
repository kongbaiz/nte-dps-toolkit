use tauri::{PhysicalPosition, Position, WebviewWindow, window::Monitor};

const MIN_REACHABLE_WIDTH: i64 = 64;
const TITLE_STRIP_HEIGHT: i64 = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PhysicalBounds {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

pub(crate) fn restore_window_position(
    window: &WebviewWindow,
    saved_position: [i32; 2],
) -> Result<(), String> {
    let monitors = window
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let Some(fallback) = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(monitor_work_area)
        .or_else(|| monitors.first().map(monitor_work_area))
    else {
        return Ok(());
    };
    let monitor_bounds: Vec<_> = monitors.iter().map(monitor_work_area).collect();
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let position = restored_window_position(
        saved_position,
        [window_size.width, window_size.height],
        &monitor_bounds,
        fallback,
    );
    window
        .set_position(Position::Physical(PhysicalPosition::new(
            position[0],
            position[1],
        )))
        .map_err(|error| error.to_string())
}

pub(crate) fn ensure_window_reachable(window: &WebviewWindow) -> Result<(), String> {
    let position = window.outer_position().map_err(|error| error.to_string())?;
    restore_window_position(window, [position.x, position.y])
}

fn monitor_work_area(monitor: &Monitor) -> PhysicalBounds {
    let work_area = monitor.work_area();
    PhysicalBounds {
        x: work_area.position.x,
        y: work_area.position.y,
        width: work_area.size.width,
        height: work_area.size.height,
    }
}

fn restored_window_position(
    saved_position: [i32; 2],
    window_size: [u32; 2],
    monitors: &[PhysicalBounds],
    fallback: PhysicalBounds,
) -> [i32; 2] {
    if monitors
        .iter()
        .any(|monitor| is_position_reachable(saved_position, window_size, *monitor))
    {
        return saved_position;
    }

    [
        centered_coordinate(fallback.x, fallback.width, window_size[0]),
        centered_coordinate(fallback.y, fallback.height, window_size[1]),
    ]
}

fn is_position_reachable(
    position: [i32; 2],
    window_size: [u32; 2],
    monitor: PhysicalBounds,
) -> bool {
    let window_left = i64::from(position[0]);
    let window_top = i64::from(position[1]);
    let window_right = window_left + i64::from(window_size[0]);
    let title_bottom = window_top + TITLE_STRIP_HEIGHT.min(i64::from(window_size[1]));
    let monitor_left = i64::from(monitor.x);
    let monitor_top = i64::from(monitor.y);
    let monitor_right = monitor_left + i64::from(monitor.width);
    let monitor_bottom = monitor_top + i64::from(monitor.height);

    let horizontal_overlap =
        (window_right.min(monitor_right) - window_left.max(monitor_left)).max(0);
    let title_overlap = (title_bottom.min(monitor_bottom) - window_top.max(monitor_top)).max(0);
    horizontal_overlap >= MIN_REACHABLE_WIDTH.min(i64::from(window_size[0]))
        && title_overlap >= TITLE_STRIP_HEIGHT.min(i64::from(window_size[1]))
}

fn centered_coordinate(origin: i32, available: u32, window: u32) -> i32 {
    let coordinate =
        i64::from(origin) + (i64::from(available).saturating_sub(i64::from(window))) / 2;
    coordinate.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_secondary_monitor_position_is_kept_when_the_title_strip_is_reachable() {
        let secondary = PhysicalBounds {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };

        assert_eq!(
            restored_window_position([-1800, 84], [760, 480], &[secondary], secondary),
            [-1800, 84]
        );
    }

    #[test]
    fn missing_monitor_position_falls_back_to_the_primary_work_area_center() {
        let primary = PhysicalBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };

        assert_eq!(
            restored_window_position([-1800, 84], [760, 480], &[primary], primary),
            [580, 280]
        );
    }

    #[test]
    fn a_partly_visible_title_strip_remains_reachable() {
        let primary = PhysicalBounds {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };

        assert_eq!(
            restored_window_position([1856, 20], [760, 480], &[primary], primary),
            [1856, 20]
        );
        assert_eq!(
            restored_window_position([1857, 20], [760, 480], &[primary], primary),
            [580, 280]
        );
    }
}
