use super::*;

pub(crate) fn draw_resource_audit_row(
    ui: &mut egui::Ui,
    item: &ResourceAuditItem,
    dark_mode: bool,
) {
    let color = match item.severity {
        ResourceAuditSeverity::Error => semantic_danger(dark_mode),
        ResourceAuditSeverity::Warning => semantic_warning(dark_mode),
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let fill = if response.hovered() {
        shadcn_card_hover(dark_mode)
    } else {
        shadcn_card(dark_mode)
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, shadcn_border(dark_mode)),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            rect.left_top(),
            egui::pos2(rect.left() + 3.0, rect.bottom()),
        ),
        6.0,
        color,
    );
    let left = rect.left() + 10.0;
    let severity_rect = egui::Rect::from_min_max(
        egui::pos2(left, rect.top()),
        egui::pos2(left + 72.0, rect.bottom()),
    );
    ui.painter().text(
        severity_rect.center(),
        egui::Align2::CENTER_CENTER,
        item.severity.label(),
        egui::FontId::proportional(11.0),
        color,
    );
    let title_left = severity_rect.right() + 8.0;
    let right_width = 172.0;
    let title_clip = egui::Rect::from_min_max(
        egui::pos2(title_left, rect.top()),
        egui::pos2(rect.right() - right_width, rect.bottom()),
    );
    let title = format!(
        "{} · {} · {}",
        item.category.label(),
        item.resource_id,
        item.display_name
    );
    ui.painter().with_clip_rect(title_clip).text(
        egui::pos2(title_left, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.0),
        shadcn_foreground(dark_mode),
    );
    ui.painter().with_clip_rect(title_clip).text(
        egui::pos2(title_left, rect.center().y + 9.0),
        egui::Align2::LEFT_CENTER,
        &item.message,
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
    ui.painter().text(
        egui::pos2(rect.right() - 10.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        &item.suggested_source,
        egui::FontId::monospace(10.0),
        ui.visuals().weak_text_color(),
    );
    response.on_hover_text(format!(
        "{}\n{}\n建议来源：{}",
        item.resource_id, item.message, item.suggested_source
    ));
}

pub(crate) fn draw_diagnostic_report(
    ui: &mut egui::Ui,
    report: &DiagnosticReport,
    dark_mode: bool,
) {
    ui.columns(3, |columns| {
        compact_metric(
            &mut columns[0],
            "失败",
            report.failed_count().to_string(),
            semantic_danger(dark_mode),
            true,
        );
        compact_metric(
            &mut columns[1],
            "警告",
            report.warning_count().to_string(),
            semantic_warning(dark_mode),
            true,
        );
        let check_color = columns[2].visuals().text_color();
        compact_metric(
            &mut columns[2],
            "检查项",
            report.checks.len().to_string(),
            check_color,
            false,
        );
    });
    ui.add_space(6.0);
    for check in &report.checks {
        let color = match check.status {
            DiagnosticStatus::Passed => semantic_success(dark_mode),
            DiagnosticStatus::Warning => semantic_warning(dark_mode),
            DiagnosticStatus::Failed => semantic_danger(dark_mode),
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::hover());
        let fill = if response.hovered() {
            shadcn_card_hover(dark_mode)
        } else {
            shadcn_card(dark_mode)
        };
        ui.painter().rect_filled(rect, 6.0, fill);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, shadcn_border(dark_mode)),
            egui::StrokeKind::Inside,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + 3.0, rect.bottom()),
            ),
            6.0,
            color,
        );
        let left = rect.left() + 10.0;
        ui.painter().text(
            egui::pos2(left, rect.center().y - 8.0),
            egui::Align2::LEFT_CENTER,
            format!("{} · {}", check.status.label(), check.title),
            egui::FontId::proportional(12.0),
            color,
        );
        ui.painter()
            .with_clip_rect(rect.shrink2(egui::vec2(10.0, 0.0)))
            .text(
                egui::pos2(left, rect.center().y + 10.0),
                egui::Align2::LEFT_CENTER,
                &check.suggestion,
                egui::FontId::proportional(10.5),
                ui.visuals().weak_text_color(),
            );
        response.on_hover_text(format!("{}\n{}", check.detail, check.suggestion));
    }
}

pub(crate) fn draw_capture_quality_summary(
    ui: &mut egui::Ui,
    summary: &CaptureQualitySummary,
    dark_mode: bool,
) {
    egui::CollapsingHeader::new(
        RichText::new("解析质量")
            .strong()
            .color(shadcn_foreground(dark_mode)),
    )
    .default_open(true)
    .show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(summary.source.label())
                    .size(12.0)
                    .color(ui.visuals().weak_text_color()),
            );
            if ui.button("复制脱敏报告").clicked() {
                ui.ctx().copy_text(summary.redacted_text());
            }
        });
        ui.add_space(4.0);
        egui::Grid::new("capture_quality_summary")
            .num_columns(4)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("封包");
                ui.monospace(format!(
                    "{} / 命中 {}",
                    summary.packet_count, summary.packets_with_hits
                ));
                ui.label("命中");
                ui.monospace(summary.hit_count.to_string());
                ui.end_row();

                ui.label("输出");
                ui.monospace(format!(
                    "{} 条 / {}",
                    summary.outgoing_hits,
                    format_number(summary.outgoing_damage)
                ));
                ui.label("候选");
                ui.monospace(format!(
                    "{} 条 / {}",
                    summary.unknown_direction_hits,
                    format_number(summary.unknown_direction_damage)
                ));
                ui.end_row();

                ui.label("受击");
                ui.monospace(format!(
                    "{} 条 / {}",
                    summary.incoming_hits,
                    format_number(summary.incoming_damage)
                ));
                ui.label("未知角色");
                ui.monospace(format!(
                    "{} 个 / {} 条",
                    summary.unknown_character_count, summary.unknown_character_hits
                ));
                ui.end_row();

                ui.label("待映射技能");
                ui.monospace(format!(
                    "{} 类 / {} 条",
                    summary.unmapped_skill_rows, summary.unmapped_skill_hits
                ));
                ui.label("未映射 GE");
                ui.monospace(summary.unmapped_gameplay_effect_count.to_string());
                ui.end_row();

                ui.label("时停");
                ui.monospace(format!(
                    "{} 事件 / {} 段",
                    summary.time_stop_event_count, summary.time_stop_interval_count
                ));
                ui.label("深渊");
                ui.monospace(format!("{} 事件", summary.abyss_event_count));
                ui.end_row();

                ui.label("伤害校准");
                ui.monospace(format!("{} 条", summary.server_damage_corrections));
                ui.label("");
                ui.label("");
                ui.end_row();
            });
    });
}
