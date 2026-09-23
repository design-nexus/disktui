use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, Dialog, Field, Focus};
use crate::model::{self, RowKind};
use crate::theme::{self, Theme};
use crate::util::{human_size, kernel_name};

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = &app.theme;
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.base()), area);
    let [body, status] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    if app.help {
        draw_help(frame, theme, body);
    } else if app.picking_theme {
        draw_themes(frame, app, theme, body);
    } else {
        draw_main(frame, app, theme, body);
    }
    if !matches!(app.dialog, Dialog::None) {
        draw_dialog(frame, app, theme, area);
    }
    draw_status(frame, app, theme, status);
}

fn draw_main(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let [list_area, detail_area] = Layout::horizontal([Constraint::Length(34), Constraint::Min(24)]).areas(area);
    draw_devices(frame, app, theme, list_area);
    if matches!(app.current_row(), Some(RowKind::Shares)) || app.focus == Focus::Shares {
        draw_shares(frame, app, theme, detail_area);
    } else {
        let [top, actions] = Layout::vertical([Constraint::Min(8), Constraint::Length(12)]).areas(detail_area);
        draw_detail(frame, app, theme, top);
        draw_actions(frame, app, theme, actions);
    }
}

fn draw_devices(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let block = panel("Devices", app.focus == Focus::Devices, theme);
    let items: Vec<ListItem> = app.rows.iter().map(|row| {
        let indent = " ".repeat(row.depth as usize * 2);
        let mark = if row.mounted { "●" } else { "○" };
        let size = if row.size.is_empty() { String::new() } else { format!("  {}", row.size) };
        let style = if row.mounted { Style::default().fg(theme.green) } else { Style::default().fg(theme.fg) };
        ListItem::new(Line::from(vec![
            Span::styled(format!("{indent}{mark} "), style),
            Span::raw(row.label.clone()),
            Span::styled(size, Style::default().fg(theme.fg_dim)),
        ]))
    }).collect();
    let list = List::new(items).block(block).highlight_style(theme.selected());
    let mut state = ListState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_detail(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let title = detail_title(app);
    let block = panel(&title, false, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [bar_area, text_area] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(inner);
    draw_bar(frame, app, theme, bar_area);
    let paragraph = Paragraph::new(detail_lines(app, theme)).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, text_area);
}

fn draw_bar(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let Some(disk) = bar_disk(app) else { return };
    let segments = app.model.segments_for(&disk);
    if segments.is_empty() || area.width == 0 {
        return;
    }
    let total: u64 = segments.iter().map(|s| s.size).sum::<u64>().max(1);
    let width = area.width as u64;
    let mut spans = Vec::new();
    let mut used = 0u16;
    for (index, segment) in segments.iter().enumerate() {
        let mut cells = ((segment.size.saturating_mul(width)) / total) as u16;
        if cells == 0 {
            cells = 1;
        }
        if index + 1 == segments.len() {
            cells = area.width.saturating_sub(used);
        }
        used = used.saturating_add(cells);
        let selected = segment_selected(app, segment);
        let bg = if segment.free { theme.bg_raised } else { theme.segment(index) };
        let fg = if segment.free { theme.muted } else { theme::on_color(bg) };
        let mut style = Style::default().bg(bg).fg(fg);
        if selected {
            style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
        }
        let label = if cells > 4 { segment.label.chars().take(cells as usize).collect::<String>() } else { String::new() };
        let mut text = format!("{label: <width$}", width = cells as usize);
        if text.chars().count() > cells as usize {
            text = text.chars().take(cells as usize).collect();
        }
        spans.push(Span::styled(text, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_actions(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let block = panel("Actions", app.focus == Focus::Actions, theme);
    let actions = app.actions();
    let items: Vec<ListItem> = actions.iter().map(|action| {
        let key = action.id.key();
        let key = if key.is_empty() { " ".into() } else { key.to_string() };
        let style = if action.enabled { Style::default().fg(theme.fg) } else { theme.dimmed() };
        let reason = if action.enabled { String::new() } else { format!("  {}", action.reason) };
        ListItem::new(Line::from(vec![
            Span::styled(format!("{key:>2}  "), Style::default().fg(theme.on_accent).bg(theme.accent)),
            Span::styled(action.id.label(), style),
            Span::styled(reason, Style::default().fg(theme.yellow)),
        ]))
    }).collect();
    let list = List::new(items).block(block).highlight_style(theme.selected());
    let mut state = ListState::default();
    if app.focus == Focus::Actions {
        state.select(Some(app.action_index));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_shares(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let block = panel("Network shares", app.focus == Focus::Shares, theme);
    let mut lines = vec![Line::from(Span::styled(
        "n new   e edit   d remove   m mount   u unmount",
        Style::default().fg(theme.muted),
    ))];
    if app.shares.is_empty() {
        lines.push(Line::from("No network shares in /etc/fstab."));
        lines.push(Line::from("Press n to add SMB, NFS, or SSHFS. Other lines in fstab are left untouched."));
    }
    for (index, share) in app.shares.iter().enumerate() {
        let style = if index == app.share_index && app.focus == Focus::Shares {
            theme.selected()
        } else {
            Style::default().fg(theme.fg)
        };
        lines.push(Line::from(Span::styled(
            format!("{:<8} {}  →  {}", share.fstype, share.source, share.target),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!("         {}", share.options),
            Style::default().fg(theme.muted),
        )));
    }
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }), area);
}

fn draw_dialog(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let rect = centered(area, 72, 20);
    frame.render_widget(Clear, rect);
    match &app.dialog {
        Dialog::Form(form) => {
            let block = panel(&form.title, true, theme);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut lines = Vec::new();
            if !form.blurb.is_empty() {
                lines.push(Line::from(Span::styled(form.blurb.clone(), Style::default().fg(theme.muted))));
                lines.push(Line::from(""));
            }
            for (index, field) in form.fields.iter().enumerate() {
                lines.push(field_line(field, index == form.focus, theme));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Enter confirms   Esc cancels   ← → changes a choice", Style::default().fg(theme.muted))));
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
        Dialog::Confirm(confirm) => {
            let block = panel(&confirm.title, true, theme);
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let mut lines = vec![
                Line::from(confirm.body.clone()),
                Line::from(""),
                Line::from(Span::styled(
                    format!("Type {} to confirm", confirm.kernel),
                    Style::default().fg(theme.yellow),
                )),
                typed_line("Device", &confirm.typed, confirm.which == 0, theme),
            ];
            if confirm.system {
                lines.push(Line::from("This is a system disk. Also type system."));
                lines.push(typed_line("Confirm", &confirm.ack, confirm.which == 1, theme));
            }
            frame.render_widget(Paragraph::new(lines), inner);
        }
        Dialog::Text { title, body, scroll } => {
            let block = panel(title, true, theme);
            frame.render_widget(
                Paragraph::new(body.as_str()).block(block).scroll((*scroll, 0)).wrap(Wrap { trim: false }),
                rect,
            );
        }
        Dialog::None => {}
    }
}

fn draw_themes(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let block = panel("Theme", true, theme);
    let items: Vec<ListItem> = theme::CHOICES.iter().enumerate().map(|(index, choice)| {
        let mark = if choice.id == app.config.theme { "● " } else { "  " };
        let preview = app.preview_name(choice.id);
        ListItem::new(Line::from(vec![
            Span::styled(mark, Style::default().fg(theme.accent)),
            Span::raw(choice.label),
            Span::styled(format!("  {preview}"), Style::default().fg(theme.muted)),
        ]))
        .style(if index == app.theme_index { theme.selected() } else { theme.base() })
    }).collect();
    let mut state = ListState::default();
    state.select(Some(app.theme_index));
    frame.render_stateful_widget(List::new(items).block(block).highlight_style(theme.selected()), area, &mut state);
}

fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect) {
    let text = "\
Devices          j/k or arrows move, Enter mounts, unmounts, or unlocks
Actions          Tab, then Enter. The letter at the left runs that action
m mount/unlock   u unmount   U force unmount   e mount options
n new partition  f format    d delete          r resize    l label
s SMART          b benchmark i image           I restore   p power off
g drive settings a attach image                t theme     x cancel job
1 disks          2 network shares              ? help      q quit

Shares           n new  e edit  d remove  m mount  u unmount
Destructive work asks you to type the kernel name. A system disk also asks for the word system.
Network shares are written to /etc/fstab with sudo. Every other line is copied through unchanged.
Esc closes a dialog. Esc during a benchmark or image copy cancels it.
a aborts a SMART self-test while the SMART page is open.
";
    frame.render_widget(Paragraph::new(text).block(panel("Keys", true, theme)).wrap(Wrap { trim: false }), area);
}

fn draw_status(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let (text, style) = if !app.error.is_empty() {
        (app.error.clone(), Style::default().fg(theme.red).bg(theme.bg_deep))
    } else if let Some(progress) = &app.progress {
        (
            format!("{}  {:3.0}%", progress.label, progress.ratio * 100.0),
            Style::default().fg(theme.accent).bg(theme.bg_deep),
        )
    } else if let Some(job) = app.model.jobs.first() {
        let pct = if job.progress_valid { format!(" {:3.0}%", job.progress * 100.0) } else { String::new() };
        (format!("{}{pct}  x cancel", job.operation), theme.status())
    } else if !app.info.is_empty() {
        (app.info.clone(), theme.status())
    } else if !app.samples.is_empty() {
        (sparkline(&app.samples, area.width.saturating_sub(2) as usize), theme.status())
    } else {
        ("? help   q quit   t theme   2 shares".into(), theme.status())
    };
    frame.render_widget(Paragraph::new(text).style(style), area);
}

fn detail_title(app: &App) -> String {
    match app.current_row() {
        Some(RowKind::Drive { drive }) => app.model.drive(&drive).map(|d| {
            format!("{} {}", d.vendor, d.model).split_whitespace().collect::<Vec<_>>().join(" ")
        }).unwrap_or_else(|| "Drive".into()),
        Some(RowKind::Volume { block }) => app.model.block(&block).map(|b| kernel_name(b.device_path()).to_string()).unwrap_or_else(|| "Volume".into()),
        Some(RowKind::Free { size, .. }) => format!("Free {}", human_size(size)),
        Some(RowKind::Raid { raid }) => app.model.raid(&raid).map(|r| format!("{} {}", r.level, r.name)).unwrap_or_else(|| "RAID".into()),
        _ => "Disks".into(),
    }
}

fn detail_lines(app: &App, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let push = |lines: &mut Vec<Line>, k: &str, v: String| {
        if v.is_empty() { return; }
        lines.push(Line::from(vec![
            Span::styled(format!("{k:<14}"), Style::default().fg(theme.muted)),
            Span::raw(v),
        ]));
    };
    match app.current_row() {
        Some(RowKind::Drive { drive }) => {
            if let Some(drive) = app.model.drive(&drive) {
                push(&mut lines, "Size", human_size(drive.size));
                push(&mut lines, "Serial", drive.serial.clone());
                push(&mut lines, "Revision", drive.revision.clone());
                push(&mut lines, "Bus", drive.connection.clone());
                push(&mut lines, "Media", drive.media.clone());
                if drive.rotation > 0 {
                    push(&mut lines, "Rotation", format!("{} rpm", drive.rotation));
                } else if drive.rotation == 0 {
                    push(&mut lines, "Rotation", "solid state".into());
                }
                if let Some(disk) = app.model.disk_for_drive(&drive.path) {
                    push(&mut lines, "Device", disk.device_path().to_string());
                    if let Some(table) = &disk.table {
                        push(&mut lines, "Table", table.kind.clone());
                    }
                }
                if let Some(ata) = &drive.ata {
                    push(&mut lines, "SMART", if !ata.smart_supported {
                        "not supported".into()
                    } else if ata.smart_failing {
                        "FAILING".into()
                    } else if ata.smart_enabled {
                        "enabled".into()
                    } else {
                        "disabled".into()
                    });
                }
                if drive.removable {
                    push(&mut lines, "Removable", "yes".into());
                }
                if drive.optical {
                    push(&mut lines, "Optical", "yes".into());
                }
                if drive.nvme {
                    push(&mut lines, "NVMe", "controller".into());
                }
                if let Some(ata) = &drive.ata {
                    if ata.security_frozen {
                        push(&mut lines, "Security", "frozen".into());
                    }
                    if ata.secure_erase_enhanced_minutes > 0 {
                        push(&mut lines, "Enhanced erase", format!("{} min", ata.secure_erase_enhanced_minutes));
                    }
                    let mut features = Vec::new();
                    if ata.apm_supported { features.push("APM"); }
                    if ata.write_cache_supported { features.push("write cache"); }
                    if ata.lookahead_supported { features.push("look-ahead"); }
                    if !features.is_empty() {
                        push(&mut lines, "Features", features.join(", "));
                    }
                }
                if app.model.disk_for_drive(&drive.path).is_some_and(|d| app.model.is_system_tree(&d.path)) {
                    push(&mut lines, "System", "yes".into());
                }
            }
        }
        Some(RowKind::Volume { block }) => {
            if let Some(block) = app.model.block(&block) {
                volume_lines(&mut lines, app, block, &push);
            }
        }
        Some(RowKind::Free { disk, start, size }) => {
            push(&mut lines, "Disk", app.model.block(&disk).map(|b| kernel_name(b.device_path()).to_string()).unwrap_or_default());
            push(&mut lines, "Start", human_size(start));
            push(&mut lines, "Size", human_size(size));
            push(&mut lines, "", "Press n to create a partition here.".into());
        }
        Some(RowKind::Raid { raid }) => {
            if let Some(raid) = app.model.raid(&raid) {
                push(&mut lines, "Level", raid.level.clone());
                push(&mut lines, "Name", raid.name.clone());
                push(&mut lines, "UUID", raid.uuid.clone());
                push(&mut lines, "Size", human_size(raid.size));
                push(&mut lines, "State", if raid.running { "running".into() } else { "stopped".into() });
                if raid.degraded { push(&mut lines, "Degraded", "yes".into()); }
                if !raid.sync_action.is_empty() {
                    push(&mut lines, "Sync", format!("{} {:.0}%", raid.sync_action, raid.sync_completed * 100.0));
                }
                push(&mut lines, "Members", raid.members.len().to_string());
            }
        }
        _ => {}
    }
    if !app.samples.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(sparkline(&app.samples, 48)));
    }
    lines
}

fn volume_lines(lines: &mut Vec<Line<'static>>, app: &App, block: &model::Block, push: &impl Fn(&mut Vec<Line>, &str, String)) {
    push(lines, "Device", block.device_path().to_string());
    push(lines, "Size", human_size(block.size));
    push(lines, "Usage", block.id_usage.clone());
    push(lines, "Type", block.id_type.clone());
    push(lines, "Label", block.id_label.clone());
    push(lines, "UUID", block.id_uuid.clone());
    if let Some(part) = &block.partition {
        push(lines, "Partition", format!("{}  {}", part.number, model::partition_type_label(&part.type_code)));
        push(lines, "Part name", part.name.clone());
        push(lines, "Part UUID", part.uuid.clone());
        push(lines, "Flags", format!("0x{:x}", part.flags));
        if part.container {
            push(lines, "Layout", "extended container".into());
        }
        if part.contained {
            push(lines, "Layout", "logical partition".into());
        }
    }
    if block.encrypted {
        push(lines, "Encryption", if block.hint_encryption.is_empty() { "LUKS".into() } else { block.hint_encryption.clone() });
    }
    if !block.mount_points.is_empty() {
        push(lines, "Mounted", block.mount_points.join(", "));
    }
    if block.swap {
        push(lines, "Swap", if block.swap_active { "active".into() } else { "inactive".into() });
    }
    if block.loopback {
        push(lines, "Backing", block.backing_file.clone());
        push(lines, "Autoclear", if block.loop_autoclear { "yes".into() } else { "no".into() });
    }
    if block.mdraid_member != "/" && !block.mdraid_member.is_empty() {
        push(lines, "RAID member", "yes".into());
    }
    if let Some(item) = block.fstab_items().into_iter().next() {
        push(lines, "fstab", model::fstab_field(item, "dir"));
        push(lines, "Options", model::fstab_field(item, "opts"));
    }
    if app.model.is_system_block(block) {
        push(lines, "System", "yes".into());
    }
}

fn field_line(field: &Field, focused: bool, theme: &Theme) -> Line<'static> {
    let value = if field.secret {
        "•".repeat(field.value.chars().count())
    } else if field.toggle {
        if field.value == "yes" { "[x]".into() } else { "[ ]".into() }
    } else if !field.choices.is_empty() {
        format!("< {} >", field.value)
    } else {
        field.value.clone()
    };
    let style = if focused { theme.selected() } else { Style::default().fg(theme.fg) };
    Line::from(vec![
        Span::styled(format!("{:<16}", field.label), Style::default().fg(theme.muted)),
        Span::styled(value, style),
    ])
}

fn typed_line(label: &str, value: &str, focused: bool, theme: &Theme) -> Line<'static> {
    let style = if focused { theme.selected() } else { Style::default().fg(theme.fg) };
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::default().fg(theme.muted)),
        Span::styled(value.to_string(), style),
    ])
}

fn sparkline(samples: &[(f64, f64)], width: usize) -> String {
    const GLYPHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    if width == 0 || samples.is_empty() {
        return String::new();
    }
    let max = samples.iter().fold(0.0_f64, |m, s| m.max(s.1)).max(1.0);
    let mut cols = vec![0.0f64; width];
    let mut seen = vec![false; width];
    for (pos, mbps) in samples {
        let index = ((*pos).clamp(0.0, 0.999) * width as f64) as usize;
        cols[index] = cols[index].max(*mbps);
        seen[index] = true;
    }
    cols.iter().zip(seen).map(|(value, seen)| {
        if !seen { " " } else {
            let slot = ((*value / max) * (GLYPHS.len() as f64 - 1.0)).round() as usize;
            GLYPHS[slot.min(GLYPHS.len() - 1)]
        }
    }).collect()
}

fn panel<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    let border = if focused { theme.accent } else { theme.border };
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .style(theme.base())
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn bar_disk(app: &App) -> Option<String> {
    match app.current_row()? {
        RowKind::Drive { drive } => app.model.disk_for_drive(&drive).map(|b| b.path.clone()),
        RowKind::Volume { block } => {
            let block = app.model.block(&block)?;
            if let Some(part) = &block.partition {
                Some(part.table.clone())
            } else {
                Some(block.path.clone())
            }
        }
        RowKind::Free { disk, .. } => Some(disk),
        _ => None,
    }
}

fn segment_selected(app: &App, segment: &model::Segment) -> bool {
    match app.current_row() {
        Some(RowKind::Volume { block }) => segment.block.as_deref() == Some(block.as_str()),
        Some(RowKind::Free { start, .. }) => segment.free && segment.start == start,
        Some(RowKind::Drive { .. }) => false,
        _ => false,
    }
}

impl App {
    pub fn current_row(&self) -> Option<RowKind> {
        self.rows.get(self.selected).map(|row| row.kind.clone())
    }

    pub fn preview_name(&self, id: &str) -> String {
        theme::resolve(id, self.omarchy.as_ref(), self.custom.as_ref()).name
    }
}
