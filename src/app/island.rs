//! Global "dynamic island" notification capsule.
//!
//! Notifications render in their own OS-level stage window instead of inside
//! each viewport: a fixed-size, fully transparent, always-on-top viewport
//! anchored to the top-center of the screen. The window itself never moves or
//! resizes while animating — the capsule morphs inside the transparent stage,
//! driven by [`motion::Spring`]s so its geometry stays continuous even when a
//! new notice retargets it mid-flight.
//!
//! Interaction model: the stage window is click-through (`WS_EX_TRANSPARENT`)
//! by default so games behind it never lose input. Hover is detected by
//! polling the global cursor against the capsule's on-screen rect (a
//! click-through window receives no mouse events), and only while the cursor
//! is over a visible capsule does the window become interactive. The window
//! additionally never activates (`WS_EX_NOACTIVATE`), so even clicking the
//! capsule keeps the game focused.

use super::*;

/// Unique window title so the island HWND can be found via `EnumWindows`
/// (eframe never exposes secondary-viewport window handles).
const ISLAND_WINDOW_TITLE: &str = "NTE Notification Island";

/// Fixed stage (window) size in logical points. Never resized at runtime:
/// per-frame `SetWindowPos`-style geometry churn is what makes overlay
/// animations stutter, so the capsule morphs inside this static canvas.
const ISLAND_STAGE_SIZE: egui::Vec2 = egui::vec2(600.0, 132.0);

const CAPSULE_TOP_MARGIN: f32 = 10.0;
const COMPACT_HEIGHT: f32 = 40.0;
const EXPANDED_HEIGHT: f32 = 76.0;
/// Entrance seed geometry: a thin sliver hugging the screen edge that the
/// capsule grows out of, echoing the hardware-cutout illusion of the original.
const SLIVER_WIDTH: f32 = 44.0;
const SLIVER_HEIGHT: f32 = 10.0;
/// The capsule rises these many points while fading in.
const ENTRANCE_RISE: f32 = 6.0;
const CAPSULE_PAD_X: f32 = 16.0;
const DOT_DIAMETER: f32 = 9.0;
const ROW_GAP: f32 = 8.0;
const MAX_TEXT_WIDTH: f32 = 400.0;
const MIN_CAPSULE_WIDTH: f32 = 132.0;
const MAX_QUEUED_NOTICES: usize = 4;
/// Width-spring impulse when a new notice replaces the current one: the
/// capsule visibly swallows the old content with a squash-and-rebound.
const REPLACE_KICK: f32 = -520.0;
/// Milder impulse when the queue advances naturally.
const ADVANCE_KICK: f32 = -220.0;
const CONTENT_FADE_SECONDS: f32 = 0.18;
/// Grace period after the cursor leaves before the dismiss timer resumes counting.
const HOVER_EXIT_GRACE: Duration = Duration::from_millis(1500);
/// Idle repaint cadence while a notice is visible: global-cursor hover polling
/// needs frames even though egui receives no mouse events while click-through.
const HOVER_POLL_INTERVAL: Duration = Duration::from_millis(90);

// The capsule is intentionally theme-independent: like its namesake it is
// always a near-black island, floating over whatever is behind it. Tone
// accents (status dot, danger border pulse) come from the live theme.
const ISLAND_FILL: Color32 = Color32::from_rgb(9, 9, 12);
const ISLAND_BORDER: Color32 = Color32::from_rgb(46, 49, 58);
const ISLAND_TEXT: Color32 = Color32::from_rgb(232, 234, 239);
const ISLAND_TEXT_MUTED: Color32 = Color32::from_rgb(150, 157, 171);
const ISLAND_CONTROL_BG: Color32 = Color32::from_rgb(24, 27, 34);
const ISLAND_CONTROL_BORDER: Color32 = Color32::from_rgb(58, 63, 77);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum IslandPhase {
    Hidden,
    Shown,
    Exiting,
}

pub(crate) struct IslandNotice {
    pub(crate) id: u64,
    pub(crate) text: String,
    pub(crate) tone: ToastTone,
    pub(crate) duration: Duration,
    pub(crate) undo_id: Option<u64>,
}

/// State machine + spring rig for the island. Owned by [`DpsApp`]; all
/// transitions that retire a notice record its id in `dropped_ids` so the app
/// can free the matching [`UndoState`] (see [`DpsApp::show_island`]).
pub(crate) struct IslandState {
    phase: IslandPhase,
    current: Option<IslandNotice>,
    queue: VecDeque<IslandNotice>,
    shown_until: Instant,
    last_tick: Instant,
    hovered: bool,
    /// Instant the visible content last changed, driving the text fade-in.
    swap_at: Instant,
    width: motion::Spring,
    height: motion::Spring,
    opacity: motion::Spring,
    /// 0..=1 reveal of the second (action) row while hovered.
    expand: motion::Spring,
    /// Danger border breathing phase in radians.
    pulse_phase: f32,
    /// Previous frame's capsule rect in desktop points, for global-cursor hit
    /// testing (the click-through window gets no egui pointer events).
    hit_rect_screen: Option<egui::Rect>,
    dropped_ids: Vec<u64>,
    hwnd: Option<isize>,
    applied_click_through: Option<bool>,
    applied_position: Option<egui::Pos2>,
}

impl IslandState {
    pub(crate) fn new() -> Self {
        let now = Instant::now();
        Self {
            phase: IslandPhase::Hidden,
            current: None,
            queue: VecDeque::new(),
            shown_until: now,
            last_tick: now,
            hovered: false,
            swap_at: now,
            width: motion::Spring::new(SLIVER_WIDTH),
            height: motion::Spring::new(SLIVER_HEIGHT),
            opacity: motion::Spring::new(0.0),
            expand: motion::Spring::new(0.0),
            pulse_phase: 0.0,
            hit_rect_screen: None,
            dropped_ids: Vec::new(),
            hwnd: None,
            applied_click_through: None,
            applied_position: None,
        }
    }

    fn is_visible(&self) -> bool {
        self.phase != IslandPhase::Hidden
    }

    pub(crate) fn push(&mut self, notice: IslandNotice) {
        let now = Instant::now();
        match self.phase {
            IslandPhase::Hidden => {
                // Grow out of a sliver hugging the screen edge.
                self.width.snap_to(SLIVER_WIDTH);
                self.height.snap_to(SLIVER_HEIGHT);
                self.opacity.snap_to(0.0);
                self.expand.snap_to(0.0);
                self.begin(notice, now);
            }
            IslandPhase::Exiting => {
                // Springs keep their current geometry and velocity, so the
                // half-collapsed capsule flows straight back open.
                self.begin(notice, now);
            }
            IslandPhase::Shown => {
                // Two push semantics: consecutive low-value status lines
                // replace each other (newest wins, with a squash impulse);
                // anything carrying tone or an undo action queues up behind
                // the current notice instead of knocking it out.
                let replace = matches!(notice.tone, ToastTone::Status)
                    && self
                        .current
                        .as_ref()
                        .is_some_and(|current| matches!(current.tone, ToastTone::Status));
                if replace {
                    if let Some(current) = self.current.take() {
                        self.dropped_ids.push(current.id);
                    }
                    self.width.kick(REPLACE_KICK);
                    self.begin(notice, now);
                } else {
                    self.queue.push_back(notice);
                    while self.queue.len() > MAX_QUEUED_NOTICES {
                        if let Some(dropped) = self.queue.pop_front() {
                            self.dropped_ids.push(dropped.id);
                        }
                    }
                }
            }
        }
    }

    fn begin(&mut self, notice: IslandNotice, now: Instant) {
        self.shown_until = now + notice.duration;
        self.swap_at = now;
        self.current = Some(notice);
        self.phase = IslandPhase::Shown;
    }

    /// Advance the timer, pausing while hovered, and dismiss on expiry.
    fn tick(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_tick);
        self.last_tick = now;
        if self.phase != IslandPhase::Shown {
            return;
        }
        if self.hovered {
            self.shown_until += elapsed;
        } else if now >= self.shown_until {
            self.dismiss_current();
        }
    }

    fn dismiss_current(&mut self) {
        if let Some(current) = &self.current {
            self.dropped_ids.push(current.id);
        }
        if let Some(next) = self.queue.pop_front() {
            self.width.kick(ADVANCE_KICK);
            self.begin(next, Instant::now());
        } else if self.phase == IslandPhase::Shown {
            // `current` stays for the exit fade; it is already reported dropped.
            self.phase = IslandPhase::Exiting;
        }
    }

    /// Remove a notice wherever it lives (undo application, external dismissal).
    pub(crate) fn remove(&mut self, id: u64) {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.id == id)
        {
            if self.phase == IslandPhase::Shown {
                self.dismiss_current();
            }
        } else {
            let before = self.queue.len();
            self.queue.retain(|notice| notice.id != id);
            if self.queue.len() != before {
                self.dropped_ids.push(id);
            }
        }
    }

    fn close_all(&mut self) {
        for notice in self.queue.drain(..) {
            self.dropped_ids.push(notice.id);
        }
        self.dismiss_current();
    }

    fn enter_hidden(&mut self) {
        self.phase = IslandPhase::Hidden;
        self.current = None;
        self.hovered = false;
        self.hit_rect_screen = None;
    }

    /// Notices newest-first, mirroring the reverse iteration the window-toast
    /// undo lookups use.
    pub(crate) fn notices_newest_first(&self) -> impl Iterator<Item = &IslandNotice> {
        self.queue.iter().rev().chain(self.current.iter())
    }

    pub(crate) fn take_dropped(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.dropped_ids)
    }

    /// Forget the native window (it is being destroyed or was destroyed by the
    /// backend, e.g. while the root viewport is minimized) so styles, position
    /// and click-through are re-applied to the replacement window.
    pub(crate) fn invalidate_window(&mut self) {
        self.hwnd = None;
        self.applied_click_through = None;
        self.applied_position = None;
    }

    /// Full reset when the island is disabled mid-run. Returns every live
    /// notice id so the caller can free their undo states.
    pub(crate) fn reset(&mut self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.current.take().map(|notice| notice.id).into_iter().collect();
        ids.extend(self.queue.drain(..).map(|notice| notice.id));
        ids.append(&mut self.dropped_ids);
        self.enter_hidden();
        self.invalidate_window();
        ids
    }

    pub(crate) fn is_idle(&self) -> bool {
        !self.is_visible() && self.queue.is_empty() && self.hwnd.is_none()
    }
}

fn island_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("nte_island_viewport")
}

/// Stage-window builder. Every field is constant so the per-frame
/// `ViewportBuilder` diff never re-sends window commands; runtime state
/// (click-through, position) is managed directly against the HWND / via
/// explicit viewport commands instead.
fn island_viewport_builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(ISLAND_WINDOW_TITLE)
        .with_inner_size(ISLAND_STAGE_SIZE)
        .with_min_inner_size(ISLAND_STAGE_SIZE)
        .with_decorations(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_resizable(false)
        // Never take focus from the game: skip activation on creation here,
        // and `apply_island_base_style` adds WS_EX_NOACTIVATE for clicks.
        .with_active(false)
        .with_taskbar(false)
        // Click-through from the very first frame; hover management takes over.
        .with_mouse_passthrough(true)
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
}

/// What the pointer did to the capsule this frame.
enum IslandAction {
    Dismiss,
    Undo(u64),
    CloseAll,
}

impl DpsApp {
    pub(crate) fn show_island(&mut self, ctx: &egui::Context) {
        if !self.island_enabled {
            if !self.island.is_idle() {
                for id in self.island.reset() {
                    self.undo_states.remove(&id);
                }
            }
            return;
        }
        ctx.show_viewport_immediate(
            island_viewport_id(),
            island_viewport_builder(),
            |ui, _class| {
                self.island_contents(ui);
            },
        );
    }

    fn island_contents(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();

        // --- native window plumbing -------------------------------------
        if self.island.hwnd.is_none()
            && let Some(hwnd) = find_process_window_by_title(ISLAND_WINDOW_TITLE)
        {
            apply_island_base_style(hwnd);
            self.island.hwnd = Some(hwnd);
        }
        // Anchor to the top-center of the window's monitor. OuterPosition goes
        // through the winit event loop between frames — never SetWindowPos from
        // inside the frame callback, which re-enters `logic()` via WndProc.
        if let Some(monitor) = ctx
            .input(|input| input.viewport().monitor_size)
            .filter(|size| size.x > 0.0)
        {
            let target = egui::pos2(
                ((monitor.x - ISLAND_STAGE_SIZE.x) * 0.5 + self.island_offset_x).round(),
                0.0,
            );
            if self
                .island
                .applied_position
                .is_none_or(|applied| applied.distance(target) > 0.5)
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target));
                self.island.applied_position = Some(target);
            }
        }

        // --- global-cursor hover + state tick ----------------------------
        let pixels_per_point = ctx.pixels_per_point();
        let cursor_points = cursor_screen_pos().map(|(x, y)| {
            egui::pos2(x as f32 / pixels_per_point, y as f32 / pixels_per_point)
        });
        let was_hovered = self.island.hovered;
        let hovered = self.island.phase == IslandPhase::Shown
            && cursor_points
                .zip(self.island.hit_rect_screen)
                .is_some_and(|(pos, rect)| rect.contains(pos));
        if was_hovered && !hovered && self.island.phase == IslandPhase::Shown {
            // Leaving the capsule grants a short grace period before expiry.
            let grace = now + HOVER_EXIT_GRACE;
            if self.island.shown_until < grace {
                self.island.shown_until = grace;
            }
        }
        self.island.hovered = hovered;
        self.island.tick(now);
        for id in self.island.take_dropped() {
            self.undo_states.remove(&id);
        }

        // --- click-through gating ----------------------------------------
        // Interactive only while a shown capsule is actually hovered; in every
        // other state (hidden, exiting, cursor elsewhere) clicks fall through
        // to whatever is behind the stage.
        let interactive = hovered && self.island.phase == IslandPhase::Shown;
        if let Some(hwnd) = self.island.hwnd
            && self.island.applied_click_through != Some(!interactive)
        {
            set_island_click_through(hwnd, !interactive);
            self.island.applied_click_through = Some(!interactive);
        }

        if self.island.phase == IslandPhase::Hidden {
            return;
        }

        // --- render-data snapshot ------------------------------------------
        let Some((text, tone, undo_id)) = self
            .island
            .current
            .as_ref()
            .map(|notice| (notice.text.clone(), notice.tone, notice.undo_id))
        else {
            self.island.enter_hidden();
            return;
        };
        let queued = self.island.queue.len();
        let theme = self.theme();
        let tone_color = match tone {
            ToastTone::Status => status_color(&text, self.paused, true),
            ToastTone::Success => theme.success,
            ToastTone::Warning => theme.warning,
            ToastTone::Danger => theme.danger,
        };

        // --- measure content, then morph (never the other way round) -------
        let text_font = egui::FontId::proportional(13.0);
        let hint_font = egui::FontId::proportional(11.0);
        let button_font = egui::FontId::proportional(11.5);
        let text_galley = ui.fonts_mut(|fonts| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                text.clone(),
                text_font.clone(),
                ISLAND_TEXT,
            );
            job.wrap.max_width = MAX_TEXT_WIDTH;
            job.wrap.max_rows = 1;
            fonts.layout_job(job)
        });
        let bubble_galley = (queued > 0).then(|| {
            ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    format!("+{queued}"),
                    egui::FontId::proportional(10.5),
                    ISLAND_TEXT_MUTED,
                )
            })
        });
        let hint_galley = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                t("Timer paused while hovering"),
                hint_font.clone(),
                ISLAND_TEXT_MUTED,
            )
        });
        let undo_label = t("Undo");
        let close_label = t("Close");
        let button_width = |label: &str, ui: &mut egui::Ui| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.to_owned(), button_font.clone(), ISLAND_TEXT)
                    .size()
                    .x
            }) + 24.0
        };
        let undo_button_width = undo_id.map(|_| button_width(&undo_label, ui));
        let close_button_width = button_width(&close_label, ui);

        let bubble_width = bubble_galley
            .as_ref()
            .map(|galley| galley.size().x + 14.0);
        let row1_width = CAPSULE_PAD_X * 2.0
            + DOT_DIAMETER
            + ROW_GAP
            + text_galley.size().x
            + bubble_width.map_or(0.0, |width| ROW_GAP + width);
        let row2_width = CAPSULE_PAD_X * 2.0
            + hint_galley.size().x
            + ROW_GAP
            + undo_button_width.map_or(0.0, |width| width + ROW_GAP)
            + close_button_width;

        let stage = ui.max_rect();
        let expanded = hovered;
        let (target_width, target_height, target_opacity, target_expand) =
            if self.island.phase == IslandPhase::Exiting {
                (SLIVER_WIDTH, SLIVER_HEIGHT, 0.0, 0.0)
            } else {
                let width = if expanded {
                    row1_width.max(row2_width)
                } else {
                    row1_width
                };
                (
                    width.max(MIN_CAPSULE_WIDTH).min(stage.width() - 16.0),
                    if expanded {
                        EXPANDED_HEIGHT
                    } else {
                        COMPACT_HEIGHT
                    },
                    1.0,
                    if expanded { 1.0 } else { 0.0 },
                )
            };
        self.island.width.set_target(target_width);
        self.island.height.set_target(target_height);
        self.island.opacity.set_target(target_opacity);
        self.island.expand.set_target(target_expand);

        let dt = ctx.input(|input| input.stable_dt).min(0.05);
        if self.reduce_motion {
            self.island.width.snap_to(target_width);
            self.island.height.snap_to(target_height);
            self.island.opacity.snap_to(target_opacity);
            self.island.expand.snap_to(target_expand);
        } else {
            // Width carries the personality: bouncy on approach so expansion
            // overshoots a touch and replacement squashes visibly. Exit and
            // the row reveal settle without overshoot, like the original.
            let width_damping = if self.island.phase == IslandPhase::Exiting {
                motion::spring::DAMPING_SMOOTH
            } else {
                motion::spring::DAMPING_BOUNCY
            };
            self.island
                .width
                .step(dt, motion::spring::STIFFNESS, width_damping);
            self.island
                .height
                .step(dt, motion::spring::STIFFNESS, width_damping);
            self.island
                .opacity
                .step(dt, motion::spring::STIFFNESS, motion::spring::DAMPING_SMOOTH);
            self.island
                .expand
                .step(dt, motion::spring::STIFFNESS, motion::spring::DAMPING_SMOOTH);
        }

        if self.island.phase == IslandPhase::Exiting
            && self.island.opacity.is_settled()
            && self.island.opacity.value() < 0.02
        {
            self.island.enter_hidden();
            return;
        }

        // --- paint -----------------------------------------------------------
        let opacity = self.island.opacity.value().clamp(0.0, 1.0);
        let width = self.island.width.value().clamp(8.0, stage.width());
        let height = self
            .island
            .height
            .value()
            .clamp(4.0, EXPANDED_HEIGHT + 12.0);
        let expand_alpha = self.island.expand.value().clamp(0.0, 1.0);
        let top = stage.top() + CAPSULE_TOP_MARGIN - (1.0 - opacity) * ENTRANCE_RISE;
        let capsule = egui::Rect::from_min_size(
            egui::pos2(stage.center().x - width * 0.5, top),
            egui::vec2(width, height),
        );
        let radius = (height * 0.5).min(20.0);

        ui.set_opacity(opacity);
        let painter = ui.painter().clone();
        painter.rect_filled(capsule, radius, ISLAND_FILL);
        let border_color = if matches!(tone, ToastTone::Danger)
            && self.island.phase == IslandPhase::Shown
            && !self.reduce_motion
        {
            self.island.pulse_phase += dt * 3.0;
            let glow = 0.5 + 0.5 * self.island.pulse_phase.sin();
            mix_color(ISLAND_BORDER, tone_color, 0.25 + 0.55 * glow)
        } else {
            ISLAND_BORDER
        };
        painter.rect_stroke(
            capsule,
            radius,
            Stroke::new(1.0_f32, border_color),
            egui::StrokeKind::Inside,
        );

        // Background click dismisses; drawn-later widgets win their own clicks.
        let mut action = None;
        let background = ui.interact(
            capsule,
            egui::Id::new("island_capsule"),
            egui::Sense::click(),
        );
        if background.clicked() {
            action = Some(IslandAction::Dismiss);
        }

        let content_painter = painter.with_clip_rect(capsule);
        let row1_center_y = capsule.top() + COMPACT_HEIGHT * 0.5;
        let mut cursor_x = capsule.left() + CAPSULE_PAD_X;
        let swap_alpha = if self.reduce_motion {
            1.0
        } else {
            (now.saturating_duration_since(self.island.swap_at).as_secs_f32()
                / CONTENT_FADE_SECONDS)
                .clamp(0.0, 1.0)
        };
        let dot_center = egui::pos2(cursor_x + DOT_DIAMETER * 0.5, row1_center_y);
        content_painter.circle_filled(dot_center, 8.0, tone_color.gamma_multiply(0.20));
        content_painter.circle_filled(dot_center, DOT_DIAMETER * 0.5, tone_color);
        cursor_x += DOT_DIAMETER + ROW_GAP;
        let text_color = ISLAND_TEXT.gamma_multiply(0.25 + 0.75 * swap_alpha);
        content_painter.galley_with_override_text_color(
            egui::pos2(cursor_x, row1_center_y - text_galley.size().y * 0.5),
            text_galley,
            text_color,
        );
        if let (Some(bubble_galley), Some(bubble_width)) = (bubble_galley, bubble_width) {
            let bubble_rect = egui::Rect::from_center_size(
                egui::pos2(
                    capsule.right() - CAPSULE_PAD_X - bubble_width * 0.5,
                    row1_center_y,
                ),
                egui::vec2(bubble_width, 18.0),
            );
            content_painter.rect_filled(bubble_rect, 9.0, ISLAND_CONTROL_BG);
            content_painter.galley(
                bubble_rect.center() - bubble_galley.size() * 0.5,
                bubble_galley,
                ISLAND_TEXT_MUTED,
            );
        }

        if expand_alpha > 0.01 {
            let row2_rect = egui::Rect::from_min_max(
                egui::pos2(
                    capsule.left() + CAPSULE_PAD_X,
                    capsule.top() + COMPACT_HEIGHT - 4.0,
                ),
                egui::pos2(capsule.right() - CAPSULE_PAD_X, capsule.bottom() - 6.0),
            );
            let mut row2 = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row2_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            row2.set_clip_rect(capsule);
            row2.multiply_opacity(expand_alpha);
            row2.spacing_mut().item_spacing.x = ROW_GAP;
            row2.label(
                RichText::new(t("Timer paused while hovering"))
                    .font(hint_font)
                    .color(ISLAND_TEXT_MUTED),
            );
            let island_button = |label: &str| {
                egui::Button::new(
                    RichText::new(label)
                        .font(button_font.clone())
                        .color(ISLAND_TEXT),
                )
                .fill(ISLAND_CONTROL_BG)
                .stroke(Stroke::new(0.5_f32, ISLAND_CONTROL_BORDER))
                .corner_radius(12)
                .min_size(egui::vec2(0.0, 24.0))
            };
            if let Some(undo_id) = undo_id
                && row2.add(island_button(&undo_label)).clicked()
            {
                action = Some(IslandAction::Undo(undo_id));
            }
            if row2.add(island_button(&close_label)).clicked() {
                action = Some(IslandAction::CloseAll);
            }
        }

        // Publish this frame's hit rect (in desktop points) for the next
        // frame's global-cursor test, with a little grace margin.
        let inner_rect = ctx.input(|input| input.viewport().inner_rect);
        self.island.hit_rect_screen = (self.island.phase == IslandPhase::Shown)
            .then(|| inner_rect.map(|inner| capsule.translate(inner.min.to_vec2()).expand(4.0)))
            .flatten();

        match action {
            Some(IslandAction::Dismiss) => self.island.dismiss_current(),
            Some(IslandAction::Undo(id)) => self.apply_undo(id, egui::ViewportId::ROOT),
            Some(IslandAction::CloseAll) => self.island.close_all(),
            None => {}
        }
        for id in self.island.take_dropped() {
            self.undo_states.remove(&id);
        }

        // --- repaint scheduling -----------------------------------------------
        let springs_settled = self.island.width.is_settled()
            && self.island.height.is_settled()
            && self.island.opacity.is_settled()
            && self.island.expand.is_settled();
        let danger_pulsing = matches!(tone, ToastTone::Danger)
            && self.island.phase == IslandPhase::Shown
            && !self.reduce_motion;
        if !springs_settled || swap_alpha < 1.0 || danger_pulsing {
            ctx.request_repaint();
        } else if self.island.phase == IslandPhase::Shown {
            let until_expiry = self
                .island
                .shown_until
                .saturating_duration_since(now)
                .min(HOVER_POLL_INTERVAL);
            ctx.request_repaint_after(until_expiry);
        }
    }
}

#[cfg(test)]
mod island_tests {
    use super::*;

    fn notice(id: u64, tone: ToastTone) -> IslandNotice {
        IslandNotice {
            id,
            text: format!("notice {id}"),
            tone,
            duration: Duration::from_secs(4),
            undo_id: None,
        }
    }

    #[test]
    fn first_notice_enters_from_the_hidden_sliver() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        assert_eq!(island.phase, IslandPhase::Shown);
        assert_eq!(island.width.value(), SLIVER_WIDTH);
        assert_eq!(island.opacity.value(), 0.0);
    }

    #[test]
    fn status_notices_replace_each_other_and_report_the_dropped_id() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.push(notice(2, ToastTone::Status));
        assert_eq!(island.current.as_ref().map(|n| n.id), Some(2));
        assert!(island.queue.is_empty());
        assert_eq!(island.take_dropped(), vec![1]);
    }

    #[test]
    fn toned_notices_queue_behind_the_current_one() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.push(notice(2, ToastTone::Success));
        island.push(notice(3, ToastTone::Warning));
        assert_eq!(island.current.as_ref().map(|n| n.id), Some(1));
        assert_eq!(island.queue.len(), 2);
        assert!(island.take_dropped().is_empty());
    }

    #[test]
    fn queue_cap_drops_the_oldest_pending_notice() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        for id in 2..=7 {
            island.push(notice(id, ToastTone::Success));
        }
        assert_eq!(island.queue.len(), MAX_QUEUED_NOTICES);
        assert_eq!(island.take_dropped(), vec![2, 3]);
    }

    #[test]
    fn dismissing_advances_the_queue_then_exits() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.push(notice(2, ToastTone::Success));
        island.dismiss_current();
        assert_eq!(island.phase, IslandPhase::Shown);
        assert_eq!(island.current.as_ref().map(|n| n.id), Some(2));
        island.dismiss_current();
        assert_eq!(island.phase, IslandPhase::Exiting);
        assert_eq!(island.take_dropped(), vec![1, 2]);
    }

    #[test]
    fn pushing_while_exiting_reopens_without_reseeding_geometry() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.width.snap_to(300.0);
        island.dismiss_current();
        assert_eq!(island.phase, IslandPhase::Exiting);
        island.push(notice(2, ToastTone::Status));
        assert_eq!(island.phase, IslandPhase::Shown);
        // Geometry continues from the mid-exit value instead of snapping
        // back to the sliver.
        assert_eq!(island.width.value(), 300.0);
    }

    #[test]
    fn hover_pauses_the_dismiss_timer() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.shown_until = Instant::now() - Duration::from_millis(1);
        island.hovered = true;
        island.tick(Instant::now());
        assert_eq!(island.phase, IslandPhase::Shown);
        island.hovered = false;
        island.shown_until = Instant::now() - Duration::from_millis(1);
        island.tick(Instant::now());
        assert_eq!(island.phase, IslandPhase::Exiting);
    }

    #[test]
    fn removing_a_queued_notice_reports_it_dropped() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.push(notice(2, ToastTone::Success));
        island.remove(2);
        assert!(island.queue.is_empty());
        assert_eq!(island.take_dropped(), vec![2]);
        assert_eq!(island.phase, IslandPhase::Shown);
    }

    #[test]
    fn reset_returns_every_live_notice_id() {
        let mut island = IslandState::new();
        island.push(notice(1, ToastTone::Status));
        island.push(notice(2, ToastTone::Success));
        island.push(notice(3, ToastTone::Warning));
        let mut ids = island.reset();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(island.phase, IslandPhase::Hidden);
        assert!(island.is_idle());
    }
}
