use super::*;

pub(crate) mod dur {
    /// Hover, press, and selection feedback.
    pub(crate) const FAST: f32 = 0.10;
    /// Standard control and value transitions.
    pub(crate) const BASE: f32 = 0.18;
    /// Viewport, mode, and other emphasized transitions.
    pub(crate) const SLOW: f32 = 0.28;
    /// Short-lived trend feedback for a changing aggregate value.
    pub(crate) const TREND: f32 = 0.80;
}

pub(crate) mod ease {
    pub(crate) fn standard(value: f32) -> f32 {
        let value = value.clamp(0.0, 1.0);
        value * value * (3.0 - 2.0 * value)
    }

    pub(crate) fn entrance(value: f32) -> f32 {
        1.0 - (1.0 - value.clamp(0.0, 1.0)).powi(3)
    }

    pub(crate) fn exit(value: f32) -> f32 {
        value.clamp(0.0, 1.0).powi(3)
    }
}

fn viewport_id(ctx: &egui::Context, id: egui::Id) -> egui::Id {
    id.with(ctx.viewport_id())
}

fn animate_bool_id(
    ctx: &egui::Context,
    id: egui::Id,
    target: bool,
    seconds: f32,
    reduce_motion: bool,
    easing: fn(f32) -> f32,
) -> f32 {
    let value =
        ctx.animate_bool_with_time_and_easing(id, target, duration(reduce_motion, seconds), easing);
    if reduce_motion {
        f32::from(target)
    } else {
        value
    }
}

fn animate_linear_value_id(
    ctx: &egui::Context,
    id: egui::Id,
    target: f32,
    seconds: f32,
    reduce_motion: bool,
) -> f32 {
    let value = ctx.animate_value_with_time(id, target, duration(reduce_motion, seconds));
    if reduce_motion { target } else { value }
}

#[derive(Clone, Copy)]
struct AnimatedValueState {
    from: f32,
    target: f32,
    current: f32,
}

impl AnimatedValueState {
    fn new(value: f32) -> Self {
        Self {
            from: value,
            target: value,
            current: value,
        }
    }
}

fn seed_animated_value_id(ctx: &egui::Context, id: egui::Id, value: f32) {
    ctx.data_mut(|data| {
        data.insert_temp(
            id.with("animated_value_state"),
            AnimatedValueState::new(value),
        )
    });
    seed_value_id(ctx, id.with("animated_value_progress"), 1.0);
}

fn animate_value_id(
    ctx: &egui::Context,
    id: egui::Id,
    target: f32,
    seconds: f32,
    reduce_motion: bool,
) -> f32 {
    let state_id = id.with("animated_value_state");
    let progress_id = id.with("animated_value_progress");
    let mut state = ctx.data_mut(|data| {
        *data.get_temp_mut_or_insert_with(state_id, || AnimatedValueState::new(target))
    });
    if state.target != target {
        state.from = state.current;
        state.target = target;
        seed_value_id(ctx, progress_id, 0.0);
    }
    let progress = animate_linear_value_id(ctx, progress_id, 1.0, seconds, reduce_motion);
    state.current = if reduce_motion {
        target
    } else {
        egui::lerp(state.from..=state.target, ease::standard(progress))
    };
    if progress >= 1.0 {
        state.from = target;
        state.current = target;
    }
    ctx.data_mut(|data| data.insert_temp(state_id, state));
    state.current
}

fn seed_bool_id(ctx: &egui::Context, id: egui::Id, value: bool) {
    ctx.animate_bool_with_time(id, value, 0.0);
}

fn seed_value_id(ctx: &egui::Context, id: egui::Id, value: f32) {
    ctx.animate_value_with_time(id, value, 0.0);
}

pub(crate) fn duration(reduce_motion: bool, seconds: f32) -> f32 {
    if reduce_motion { 0.0 } else { seconds }
}

pub(crate) fn animate_bool(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    target: bool,
    seconds: f32,
    reduce_motion: bool,
    easing: fn(f32) -> f32,
) -> f32 {
    animate_bool_id(
        ctx,
        viewport_id(ctx, egui::Id::new(id)),
        target,
        seconds,
        reduce_motion,
        easing,
    )
}

pub(crate) fn animate_value(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    target: f32,
    seconds: f32,
    reduce_motion: bool,
) -> f32 {
    animate_value_id(
        ctx,
        viewport_id(ctx, egui::Id::new(id)),
        target,
        seconds,
        reduce_motion,
    )
}

pub(crate) fn snap_value(ctx: &egui::Context, id: impl std::hash::Hash, value: f32) {
    seed_animated_value_id(ctx, viewport_id(ctx, egui::Id::new(id)), value);
}

pub(crate) fn seed_bool(ctx: &egui::Context, id: impl std::hash::Hash, value: bool) {
    seed_bool_id(ctx, viewport_id(ctx, egui::Id::new(id)), value);
}

pub(crate) fn seed_bool_for_viewport(
    ctx: &egui::Context,
    viewport: egui::ViewportId,
    id: impl std::hash::Hash,
    value: bool,
) {
    seed_bool_id(ctx, egui::Id::new(id).with(viewport), value);
}

pub(crate) fn animate_generation(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    generation: u32,
    seconds: f32,
    reduce_motion: bool,
) -> f32 {
    let target = generation as f32;
    let animated = animate_value(ctx, id, target, seconds, reduce_motion);
    (target - animated).clamp(0.0, 1.0)
}

pub(crate) fn bounce_envelope(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    4.0 * value * (1.0 - value)
}

pub(crate) fn apply_viewport_entrance(
    ui: &mut egui::Ui,
    id: &'static str,
    opening: bool,
    reduce_motion: bool,
) {
    let animation_id = ("viewport_entrance", id);
    if opening {
        seed_bool(ui.ctx(), animation_id, false);
    }
    let progress = animate_bool(
        ui.ctx(),
        animation_id,
        true,
        dur::SLOW,
        reduce_motion,
        ease::entrance,
    );
    ui.set_opacity(progress);
    ui.add_space((1.0 - progress) * 8.0);
}

/// Fade-and-rise entrance replayed whenever `key` changes — used for
/// in-window page swaps such as switching console tabs. The first key a fresh
/// window renders gets no animation so it doesn't stack with
/// [`apply_viewport_entrance`].
pub(crate) fn content_swap_entrance(
    ui: &mut egui::Ui,
    id: &'static str,
    key: u64,
    reduce_motion: bool,
) {
    let ctx = ui.ctx().clone();
    let base = viewport_id(&ctx, egui::Id::new(("content_swap", id)));
    let key_id = base.with("key");
    let progress_id = base.with("progress");
    let previous = ctx.data_mut(|data| data.get_temp::<u64>(key_id));
    if previous != Some(key) {
        ctx.data_mut(|data| data.insert_temp(key_id, key));
        if previous.is_some() {
            seed_value_id(&ctx, progress_id, 0.0);
        }
    }
    let progress = ease::entrance(animate_linear_value_id(
        &ctx,
        progress_id,
        1.0,
        dur::BASE,
        reduce_motion,
    ));
    ui.set_opacity(progress);
    ui.add_space((1.0 - progress) * 6.0);
}

#[derive(Clone, Copy)]
struct RollingValueState {
    from: f64,
    target: f64,
    current: f64,
}

impl RollingValueState {
    fn from_zero(target: f64) -> Self {
        Self {
            from: 0.0,
            target,
            current: 0.0,
        }
    }
}

fn interpolate_f64(from: f64, target: f64, progress: f32) -> f64 {
    if progress >= 1.0 {
        target
    } else {
        from + (target - from) * f64::from(progress.clamp(0.0, 1.0))
    }
}

/// Animate a normalized progress value while retaining the metric itself as `f64`.
/// The exact target is returned on completion instead of a lossy `f32` round-trip.
pub(crate) fn rolling_value(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    target: f64,
    seconds: f32,
    reduce_motion: bool,
) -> f64 {
    let id = viewport_id(ctx, egui::Id::new(id));
    let state_id = id.with("rolling_state");
    let progress_id = id.with("rolling_progress");
    let mut state =
        if let Some(state) = ctx.data(|data| data.get_temp::<RollingValueState>(state_id)) {
            state
        } else {
            seed_value_id(ctx, progress_id, 0.0);
            RollingValueState::from_zero(target)
        };

    if state.target != target {
        state.from = state.current;
        state.target = target;
        seed_value_id(ctx, progress_id, 0.0);
    }

    let progress = animate_linear_value_id(ctx, progress_id, 1.0, seconds, reduce_motion);
    state.current = if reduce_motion {
        target
    } else {
        interpolate_f64(state.from, state.target, ease::standard(progress))
    };
    if progress >= 1.0 {
        state.from = target;
        state.current = target;
    }
    ctx.data_mut(|data| data.insert_temp(state_id, state));
    state.current
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrendDirection {
    Up,
    Down,
}

#[derive(Clone, Copy)]
pub(crate) struct TrendIndicator {
    pub(crate) direction: TrendDirection,
    pub(crate) opacity: f32,
}

#[derive(Clone, Copy)]
struct TrendState {
    target: f64,
    direction: Option<TrendDirection>,
    active: bool,
}

fn trend_direction(previous: f64, target: f64) -> Option<TrendDirection> {
    if target > previous {
        Some(TrendDirection::Up)
    } else if target < previous {
        Some(TrendDirection::Down)
    } else {
        None
    }
}

pub(crate) fn trend_indicator(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    target: f64,
    reduce_motion: bool,
) -> Option<TrendIndicator> {
    let id = viewport_id(ctx, egui::Id::new(id));
    let state_id = id.with("trend_state");
    let progress_id = id.with("trend_progress");
    let mut state = ctx.data_mut(|data| {
        *data.get_temp_mut_or_insert_with(state_id, || TrendState {
            target,
            direction: None,
            active: false,
        })
    });

    if state.target != target {
        state.direction = trend_direction(state.target, target);
        state.target = target;
        state.active = state.direction.is_some() && !reduce_motion;
        seed_value_id(ctx, progress_id, 0.0);
    }

    let progress = animate_linear_value_id(ctx, progress_id, 1.0, dur::TREND, reduce_motion);
    let indicator = state.active.then(|| TrendIndicator {
        direction: state
            .direction
            .expect("active trend always has a direction"),
        opacity: 1.0 - ease::standard(progress),
    });
    if progress >= 1.0 {
        state.active = false;
    }
    ctx.data_mut(|data| data.insert_temp(state_id, state));
    indicator.filter(|indicator| indicator.opacity > 0.0)
}

#[derive(Clone, Copy)]
struct ShareState {
    target: f32,
    highlight_active: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct ShareAnimation {
    pub(crate) value: f32,
    pub(crate) highlight_opacity: f32,
}

fn share_grew(previous: f32, target: f32) -> bool {
    target > previous
}

pub(crate) fn animate_share(
    ctx: &egui::Context,
    id: impl std::hash::Hash,
    target: f32,
    reduce_motion: bool,
) -> ShareAnimation {
    let id = viewport_id(ctx, egui::Id::new(id));
    let state_id = id.with("share_state");
    let value_id = id.with("share_value");
    let highlight_id = id.with("share_highlight");
    let state = ctx.data(|data| data.get_temp::<ShareState>(state_id));
    let mut state = state.unwrap_or_else(|| {
        seed_animated_value_id(ctx, value_id, 0.0);
        seed_value_id(ctx, highlight_id, 0.0);
        ShareState {
            target,
            highlight_active: target > 0.0 && !reduce_motion,
        }
    });

    if state.target != target {
        state.highlight_active = share_grew(state.target, target) && !reduce_motion;
        state.target = target;
        seed_value_id(ctx, highlight_id, 0.0);
    }

    let value = animate_value_id(ctx, value_id, target, dur::BASE, reduce_motion);
    let highlight_progress =
        animate_linear_value_id(ctx, highlight_id, 1.0, dur::BASE, reduce_motion);
    let highlight_opacity = if state.highlight_active {
        1.0 - ease::standard(highlight_progress)
    } else {
        0.0
    };
    if highlight_progress >= 1.0 {
        state.highlight_active = false;
    }
    ctx.data_mut(|data| data.insert_temp(state_id, state));

    ShareAnimation {
        value,
        highlight_opacity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_curves_keep_their_endpoints() {
        for easing in [ease::standard, ease::entrance, ease::exit] {
            assert_eq!(easing(0.0), 0.0);
            assert_eq!(easing(1.0), 1.0);
        }
    }

    #[test]
    fn reduced_motion_zeroes_duration() {
        assert_eq!(duration(true, dur::SLOW), 0.0);
        assert_eq!(duration(false, dur::BASE), dur::BASE);
    }

    #[test]
    fn f64_interpolation_returns_the_exact_target_at_completion() {
        let target = 9_007_199_254_740_991.0;
        assert_eq!(interpolate_f64(0.0, target, 1.0), target);
    }

    #[test]
    fn trend_direction_only_reports_real_changes() {
        assert_eq!(trend_direction(10.0, 11.0), Some(TrendDirection::Up));
        assert_eq!(trend_direction(10.0, 9.0), Some(TrendDirection::Down));
        assert_eq!(trend_direction(10.0, 10.0), None);
    }

    #[test]
    fn share_highlight_only_triggers_on_growth() {
        assert!(share_grew(0.25, 0.5));
        assert!(!share_grew(0.5, 0.25));
        assert!(!share_grew(0.5, 0.5));
    }

    #[test]
    fn bounce_envelope_starts_and_ends_at_rest() {
        assert_eq!(bounce_envelope(0.0), 0.0);
        assert_eq!(bounce_envelope(0.5), 1.0);
        assert_eq!(bounce_envelope(1.0), 0.0);
    }
}
