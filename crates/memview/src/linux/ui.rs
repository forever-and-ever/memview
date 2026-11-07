use super::activity::Activity;
use super::app::{App, FlatProcessRow, FlatSharedRow, FlatTmpfsRow, RowFold};
use super::model::{
    Bytes, Meminfo, MeminfoEntry, NvidiaPoolLedger, ObjectUsage, PhysicalResidual, Pid,
    ProcessCoverage, ProcessMemory, ProcessTotals, Processes, Shared, TmpfsMount,
};
use super::search::SearchRole;
use libshimmer::Motion;
use libshimmer_ratatui::RowExt as _;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};
use std::borrow::Cow;
use std::time::Duration;
use tui_eternal::palette::{ACCENT, BG, CURSOR, FG, GOLD, HOT, MUTED, PAGE, SHIMMER};
use tui_eternal::{badge, legend, masthead, panel, tally};

pub fn render(frame: &mut Frame<'_>, app: &App, activity: &Activity) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(header(app), chunks[0]);
    render_activity(frame, app, activity, chunks[0]);
    if app.active_ledger_ready() {
        render_body(frame, app, chunks[1]);
    } else {
        render_loading(frame, app, chunks[1]);
    }
    frame.render_widget(footer(app), chunks[2]);

    if let Some(help) = app.help() {
        help.render(frame, &app.sections(), NOTES);
    }
    if app.kill_confirmation().is_some() {
        render_kill_confirmation(frame, app, area);
    }
    if app.search_draft().is_some() {
        render_search_prompt(frame, app, area);
    }
}

fn render_body(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.tab {
        super::app::Tab::Overview => render_overview(frame, app, area),
        super::app::Tab::Processes => render_processes(frame, app, area),
        super::app::Tab::Tmpfs => render_tmpfs(frame, app, area),
        super::app::Tab::Shared => render_shared(frame, app, area),
    }
}

fn header(app: &App) -> Paragraph<'static> {
    Paragraph::new(header_line(app))
        .block(masthead())
        .style(PAGE)
}

fn header_line(app: &App) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(badge("memview"));
    spans.push(Span::raw("  "));
    for (index, label) in App::tab_labels().into_iter().enumerate() {
        let style = if app.tab == super::app::Tab::ALL[index] {
            ACCENT.fg().add_modifier(Modifier::BOLD)
        } else {
            MUTED.fg()
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(
        format!("sort {}", app.metric.label()),
        GOLD.fg(),
    ));
    if matches!(app.tab, super::app::Tab::Processes | super::app::Tab::Tmpfs) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("mode {}", app.tree_scope.label()),
            ACCENT.fg(),
        ));
    }
    Line::from(spans)
}

/// The activity readout, right-aligned in the header bar: the scan under way, dressed by the
/// shimmer, then its elapsed time. Absent while no scan runs, and when the bar lacks room.
fn render_activity(frame: &mut Frame<'_>, app: &App, activity: &Activity, area: Rect) {
    let Some(running) = activity.running(app) else {
        return;
    };
    let row = activity.shimmer().status(
        SHIMMER,
        Motion {
            mark: true,
            letters: true,
        },
    );
    let mut spans = row.spans();
    spans.push(Span::styled(
        format!("  {} ms", running.as_millis()),
        MUTED.fg(),
    ));
    let line = Line::from(spans);
    let bar = masthead().inner(area);
    let width = u16::try_from(line.width()).unwrap_or(u16::MAX);
    let taken = u16::try_from(header_line(app).width()).unwrap_or(u16::MAX);
    if taken.saturating_add(2).saturating_add(width) > bar.width {
        return;
    }
    let lead = Rect {
        x: bar.right() - width,
        width,
        ..bar
    };
    frame.render_widget(Paragraph::new(line).style(PAGE), lead);
    row.shade(lead, frame.buffer_mut());
}

fn footer(app: &App) -> Paragraph<'_> {
    let mut spans = Vec::new();
    if app.tab.drives_process_scans()
        && let Some(reason) = app.refresh_pause()
    {
        spans.push(Span::styled(
            match reason {
                super::app::RefreshPause::Inactive => "PAUSED · idle · press a key to resume  ",
                super::app::RefreshPause::Unfocused => "PAUSED · terminal unfocused  ",
            },
            GOLD.fg(),
        ));
    }
    if let Some(pattern) = app.search_pattern() {
        spans.push(Span::styled(
            format!(" FILTER /{pattern}/ "),
            BG.fg().bg(GOLD.color()).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    spans.extend(legend(&app.sections()));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(app.current_time_label(), MUTED.fg()));
    if let Some(error) = &app.last_error {
        spans.push(Span::styled("  last error: ", HOT.fg()));
        spans.push(Span::styled(error.as_str(), HOT.fg()));
    }
    if let Some(confirmation) = app.kill_confirmation() {
        if confirmation.armed() {
            spans.push(Span::styled(
                "  SIGTERM armed: y confirms",
                HOT.fg().add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                format!(
                    "  SIGTERM locked {} ms",
                    confirmation.lock_remaining().as_millis()
                ),
                GOLD.fg(),
            ));
        }
    }
    Paragraph::new(Line::from(spans))
        .wrap(Wrap { trim: false })
        .style(BG.bg())
}

fn render_loading(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let (title, message) = match app.tab {
        super::app::Tab::Overview => ("Loading", "Reading kernel memory counters..."),
        super::app::Tab::Processes => ("Processes", "Capturing process memory snapshot..."),
        super::app::Tab::Tmpfs => ("Tmpfs", "Scanning tmpfs mounts..."),
        super::app::Tab::Shared => ("Shared", "Reading shared memory ledgers..."),
    };
    frame.render_widget(
        Paragraph::new(message)
            .block(panel(title, false))
            .style(FG.fg()),
        area,
    );
}

fn render_overview(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(meminfo) = app.meminfo() else {
        render_loading(frame, app, area);
        return;
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(area);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(13),
            Constraint::Length(12),
            Constraint::Min(6),
        ])
        .split(columns[1]);

    let mem_rows = meminfo
        .entries
        .iter()
        .map(|entry| row_meminfo(entry, meminfo))
        .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(
            mem_rows,
            [
                Constraint::Length(18),
                Constraint::Length(16),
                Constraint::Length(8),
            ],
        )
        .header(header_row(["Kernel Counter", "Value", "% total"]))
        .block(panel("meminfo", false))
        .column_spacing(1),
        columns[0],
    );

    let physical_rows = meminfo.physical_ledger().map_or_else(
        || {
            vec![summary_text_row(
                "Physical ledger",
                "required counters absent",
            )]
        },
        |physical| {
            let mut rows = vec![
                accounting_row("Allocated RAM", physical.allocated, physical.total),
                accounting_row("LRU pages", physical.lru, physical.total),
                accounting_row("Slab", physical.slab, physical.total),
                accounting_row("HugeTLB", physical.hugetlb, physical.total),
                accounting_row("Kernel counters", physical.kernel, physical.total),
            ];
            let nvidia = app
                .inventory()
                .map_or(NvidiaPoolLedger::Unsupported, |inventory| {
                    inventory.nvidia_pools
                });
            match physical.residual {
                PhysicalResidual::Inconsistent { excess } => rows.push(summary_text_row(
                    "Counter overlap",
                    &format!("classified exceeds allocated by {excess}"),
                )),
                PhysicalResidual::Estimate(direct) => match nvidia {
                    NvidiaPoolLedger::Exact(snapshot) => {
                        rows.push(accounting_row(
                            &format!("NVIDIA pool lens ({})", snapshot.pool_count),
                            snapshot.bytes,
                            physical.total,
                        ));
                        rows.push(accounting_row("Residual estimate", direct, physical.total));
                    }
                    NvidiaPoolLedger::Disabled => {
                        rows.push(accounting_row("Residual estimate", direct, physical.total));
                        rows.push(summary_text_row("NVIDIA sysmem pools", "disabled"));
                    }
                    NvidiaPoolLedger::Inaccessible => {
                        rows.push(accounting_row("Residual estimate", direct, physical.total));
                        rows.push(summary_text_row("NVIDIA sysmem pools", "inaccessible"));
                    }
                    NvidiaPoolLedger::Malformed => {
                        rows.push(accounting_row("Residual estimate", direct, physical.total));
                        rows.push(summary_text_row("NVIDIA sysmem pools", "malformed"));
                    }
                    NvidiaPoolLedger::Unsupported => {
                        rows.push(accounting_row("Residual estimate", direct, physical.total));
                    }
                },
            }
            rows.push(accounting_row("Free RAM", physical.free, physical.total));
            rows
        },
    );
    frame.render_widget(
        Table::new(
            physical_rows,
            [Constraint::Length(24), Constraint::Length(16)],
        )
        .header(header_row(["Physical ledger", "Value"]))
        .block(panel("RAM reconciliation", false))
        .column_spacing(1),
        right[0],
    );

    let process_totals = app
        .processes()
        .map_or_else(ProcessTotals::default, |processes| processes.totals.clone());
    let process_coverage = app
        .processes()
        .map_or_else(ProcessCoverage::default, |processes| {
            processes.tree.coverage
        });
    let tmpfs_allocated = app
        .tmpfs()
        .map_or(Bytes::ZERO, |tmpfs| tmpfs.allocated_total);
    let sysv_rss = app
        .inventory()
        .map_or(Bytes::ZERO, |inventory| inventory.sysv_rss_total);
    let sysv_segments = app
        .inventory()
        .map_or(0, |inventory| inventory.sysv_segments.len());
    let overview_rows = vec![
        summary_row(
            &format!("Σ process PSS [{}]", app.process_capture_label()),
            process_totals.pss,
        ),
        summary_row(
            &format!("Σ process USS [{}]", app.process_capture_label()),
            process_totals.uss,
        ),
        summary_row(
            &format!("Σ process RSS [{}]", app.process_capture_label()),
            process_totals.rss,
        ),
        summary_row(
            &format!("Σ process SwapPSS [{}]", app.process_capture_label()),
            process_totals.swap_pss,
        ),
        summary_row(
            &format!("Σ process PSS anon [{}]", app.process_capture_label()),
            process_totals.pss_anon,
        ),
        summary_row(
            &format!("Σ process PSS file [{}]", app.process_capture_label()),
            process_totals.pss_file,
        ),
        summary_row(
            &format!("Σ process PSS shmem [{}]", app.process_capture_label()),
            process_totals.pss_shmem,
        ),
        summary_row(
            &format!("Σ tmpfs allocated [{}]", app.tmpfs_capture_label()),
            tmpfs_allocated,
        ),
        summary_row("Σ SysV shm RSS", sysv_rss),
    ];
    frame.render_widget(
        Table::new(
            overview_rows,
            [Constraint::Length(24), Constraint::Length(16)],
        )
        .header(header_row(["Lens", "Value"]))
        .block(panel("independent attribution captures", false))
        .column_spacing(1),
        right[1],
    );

    let warnings = app.warnings();
    let warning_lines = if warnings.is_empty() {
        vec![Line::from(Span::styled(
            format!(
                "No probe warnings. Captured {}/{} processes ({} vanished, {} inaccessible), \
                 {sysv_segments} SysV segments; scan {} ms. Rollups exact/status: {}/{}; maps \
                 exact/inaccessible/deferred: {}/{}/{}. The physical residual is an estimate from \
                 selected /proc/meminfo counters, which can overlap or omit ownership.",
                process_coverage.captured,
                process_coverage.candidates,
                process_coverage.vanished,
                process_coverage.inaccessible,
                app.last_capture_elapsed().as_millis(),
                process_coverage.exact_rollups,
                process_coverage.status_rollups,
                process_coverage.exact_maps,
                process_coverage.inaccessible_maps,
                process_coverage.deferred_maps,
            ),
            FG.fg(),
        ))]
    } else {
        warnings
            .iter()
            .take(24)
            .map(|warning| Line::from(Span::styled(*warning, HOT.fg())))
            .collect::<Vec<_>>()
    };
    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(panel("probe notes", false))
            .wrap(Wrap { trim: false })
            .style(FG.fg()),
        right[2],
    );
}

fn render_processes(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(processes) = app.processes() else {
        render_loading(frame, app, area);
        return;
    };
    let capacity = processes.meminfo.value("MemTotal").unwrap_or(Bytes::ZERO);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(57), Constraint::Percentage(43)])
        .split(area);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(16), Constraint::Min(8)])
        .split(columns[1]);

    if processes.tree.nodes.is_empty() && app.process_scan_started_at.is_some() {
        frame.render_widget(
            Paragraph::new("Capturing process memory snapshot...")
                .block(panel("process tree", true))
                .style(FG.fg()),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new("Per-process PSS, mappings, and object consumers will appear here.")
                .block(panel("selected process", false))
                .wrap(Wrap { trim: false })
                .style(MUTED.fg()),
            right[0],
        );
        return;
    }

    let rows = app.process_rows();
    let selected = app.selected_process_row();
    let visible = slice_window(rows, selected, columns[0].height.saturating_sub(4) as usize);
    let process_rows = visible
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            row_process(
                app,
                processes,
                row,
                visible.start + offset == selected,
                capacity,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(
            process_rows,
            [
                Constraint::Length(30),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Min(24),
            ],
        )
        .header(header_row([
            "Task", "PID", "User", "PSS", "USS", "RSS", "Command",
        ]))
        .block(panel("process tree", true).title(tally(app.tree_scope.label())))
        .column_spacing(1),
        columns[0],
    );

    if let Some(process) = app.selected_process() {
        let mut details = search_summary_lines(app, capacity);
        details.push(Line::from(vec![Span::styled(
            process.title(),
            ACCENT.fg().add_modifier(Modifier::BOLD),
        )]));
        if let Some(cwd) = &process.cwd {
            details.push(detail_line("CWD", cwd.as_str()));
        }
        details.extend([
            detail_line("State", process.state.as_str()),
            detail_line("Threads", process.threads.to_string()),
        ]);
        match process.memory {
            ProcessMemory::Smaps(rollup) => details.extend([
                detail_line("PSS", rollup.pss.human_exact()),
                detail_line("USS", rollup.uss().human_exact()),
                detail_line("RSS", rollup.rss.human_exact()),
                detail_line("PSS anon", rollup.pss_anon.human_exact()),
                detail_line("PSS file", rollup.pss_file.human_exact()),
                detail_line("PSS shmem", rollup.pss_shmem.human_exact()),
                detail_line("SwapPSS", rollup.swap_pss.human_exact()),
            ]),
            ProcessMemory::Status(status) => details.extend([
                detail_line("PSS / USS", "unavailable"),
                detail_line("Status RSS", status.rss.human_exact()),
                detail_line("RSS anon", status.anonymous.human_exact()),
                detail_line("RSS file", status.file.human_exact()),
                detail_line("RSS shmem", status.shmem.human_exact()),
                detail_line("Status swap", status.swap.human_exact()),
            ]),
        }
        details.extend([
            detail_line(
                "Access",
                format!(
                    "rollup={} maps={}",
                    process.rollup_state().label(),
                    app.selected_process_mapping_status()
                ),
            ),
            detail_line("Map scan", app.selected_process_mapping_scan_label()),
        ]);
        frame.render_widget(
            Paragraph::new(details)
                .block(panel("selected process", false))
                .wrap(Wrap { trim: false })
                .style(FG.fg()),
            right[0],
        );
    } else if app.search_summary().is_some() {
        frame.render_widget(
            Paragraph::new(search_summary_lines(app, capacity))
                .block(panel("search total", false))
                .wrap(Wrap { trim: false })
                .style(FG.fg()),
            right[0],
        );
    }

    if let Some((pid, elapsed)) = app.selected_process_mapping_loading() {
        frame.render_widget(mapping_loading(pid, elapsed), right[1]);
    } else {
        let objects = app.selected_process_objects();
        let object_window = slice_window(objects, 0, right[1].height.saturating_sub(4) as usize);
        let object_rows = object_window
            .iter()
            .map(|object| row_object_usage(object, capacity))
            .collect::<Vec<_>>();
        frame.render_widget(
            Table::new(
                object_rows,
                [
                    Constraint::Length(9),
                    Constraint::Length(10),
                    Constraint::Length(10),
                    Constraint::Length(7),
                    Constraint::Min(24),
                ],
            )
            .header(header_row(["Kind", "PSS", "RSS", "VMAs", "Object"]))
            .block(panel("selected mappings", false))
            .column_spacing(1),
            right[1],
        );
    }
}

fn mapping_loading(pid: Pid, elapsed: Duration) -> Paragraph<'static> {
    let dots = ".".repeat(((elapsed.as_millis() / 250) % 4) as usize);
    Paragraph::new(vec![
        Line::from(vec![Span::styled(
            format!("Loading /proc/{pid}/smaps{dots}"),
            GOLD.fg().add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![
            Span::styled("elapsed ", MUTED.fg()),
            Span::styled(format!("{} ms", elapsed.as_millis()), FG.fg()),
        ]),
        Line::from(""),
        Line::from("The kernel synthesizes per-VMA PSS/RSS here; large mapping tables can stall."),
    ])
    .block(panel("selected mappings", false))
    .wrap(Wrap { trim: false })
    .style(FG.fg())
}

fn render_tmpfs(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(tmpfs) = app.tmpfs() else {
        render_loading(frame, app, area);
        return;
    };
    let capacity = app
        .meminfo()
        .and_then(|meminfo| meminfo.value("MemTotal"))
        .unwrap_or(Bytes::ZERO);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let rows = app.tmpfs_rows();
    let selected = app.selected_tmpfs_row();
    let visible = slice_window(rows, selected, columns[0].height.saturating_sub(4) as usize);
    let table_rows = visible
        .iter()
        .enumerate()
        .map(|(offset, row)| row_tmpfs(tmpfs, row, visible.start + offset == selected, capacity))
        .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(
            table_rows,
            [
                Constraint::Length(32),
                Constraint::Length(8),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Min(20),
            ],
        )
        .header(header_row([
            "Entry",
            "Kind",
            "Allocated",
            "Logical",
            "Path",
        ]))
        .block(panel("tmpfs generation", true).title(tally(&format!(
            "{}/{} filesystems; {} walk gaps",
            tmpfs.coverage.captured_filesystems,
            tmpfs.coverage.unique_filesystems,
            tmpfs.coverage.walk_errors
        ))))
        .column_spacing(1),
        columns[0],
    );

    let mut detail_lines = search_summary_lines(app, capacity);
    detail_lines.extend(
        match (app.selected_tmpfs_mount(), app.selected_tmpfs_node()) {
            (Some(mount), Some(node)) => tmpfs_detail_lines(mount, node),
            _ => vec![Line::from("No tmpfs node selected")],
        },
    );
    frame.render_widget(
        Paragraph::new(detail_lines)
            .block(panel("selected tmpfs entry", false))
            .wrap(Wrap { trim: false })
            .style(FG.fg()),
        columns[1],
    );
}

fn render_shared(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(shared) = app.shared() else {
        render_loading(frame, app, area);
        return;
    };
    let capacity = shared.meminfo.value("MemTotal").unwrap_or(Bytes::ZERO);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    let rows = app.shared_rows();
    let selected = app.selected_shared_row();
    let visible = slice_window(rows, selected, columns[0].height.saturating_sub(4) as usize);
    let rows = visible
        .iter()
        .enumerate()
        .map(|(offset, row)| {
            row_shared(
                shared,
                row,
                visible.start + offset == selected,
                capacity,
                app.metric,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(7),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(7),
                Constraint::Min(24),
            ],
        )
        .header(header_row([
            "Kind", "Tasks", "PSS", "RSS", "VMAs", "Object",
        ]))
        .block(panel("backing ledger", true).title(tally(&format!(
            "{}/{} maps exact",
            shared.coverage.exact_maps, shared.coverage.captured
        ))))
        .column_spacing(1),
        columns[0],
    );

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(8)])
        .split(columns[1]);
    if let Some(object) = app.selected_shared_object() {
        let mut summary = search_summary_lines(app, capacity);
        summary.extend([
            Line::from(Span::styled(
                object.label.as_str(),
                ACCENT.fg().add_modifier(Modifier::BOLD),
            )),
            detail_line("Kind", object.kind.label()),
            detail_line("Backing", object.backing.to_string()),
            detail_line("PSS", object.rollup.pss.human_exact()),
            detail_line("RSS", object.rollup.rss.human_exact()),
            detail_line("Swap", object.rollup.swap.human_exact()),
            detail_line("Tasks", object.mapped_processes.to_string()),
            detail_line("VMAs", object.regions.to_string()),
        ]);
        frame.render_widget(
            Paragraph::new(summary)
                .block(panel("selected object", false))
                .wrap(Wrap { trim: false })
                .style(FG.fg()),
            right[0],
        );

        let consumer_window = slice_window(
            &object.consumers,
            0,
            right[1].height.saturating_sub(4) as usize,
        );
        let consumers = consumer_window
            .iter()
            .map(|consumer| {
                Row::new(vec![
                    Cell::from(consumer.pid.to_string()),
                    usage_cell(consumer.rollup.pss, capacity),
                    usage_cell(consumer.rollup.rss, capacity),
                    Cell::from(consumer.name.as_str()),
                    Cell::from(consumer.command.as_str()),
                ])
                .style(usage_style(false, consumer.rollup.pss, capacity))
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Table::new(
                consumers,
                [
                    Constraint::Length(8),
                    Constraint::Length(10),
                    Constraint::Length(10),
                    Constraint::Length(16),
                    Constraint::Min(20),
                ],
            )
            .header(header_row(["PID", "PSS", "RSS", "Name", "Command"]))
            .block(panel("top consumers", false))
            .column_spacing(1),
            right[1],
        );
    } else if app.search_summary().is_some() {
        frame.render_widget(
            Paragraph::new(search_summary_lines(app, capacity))
                .block(panel("search total", false))
                .wrap(Wrap { trim: false })
                .style(FG.fg()),
            right[0],
        );
    }
}

fn row_meminfo<'a>(entry: &'a MeminfoEntry, meminfo: &Meminfo) -> Row<'a> {
    let total = meminfo.value("MemTotal").unwrap_or(Bytes::ZERO);
    let color = MeminfoTone::for_key(&entry.key).color(entry.value, meminfo);
    Row::new(vec![
        Cell::from(entry.key.as_str()),
        Cell::from(entry.value.human_iec()).style(Style::default().fg(color)),
        Cell::from(format!("{:.1}", entry.value.pct_of(total))).style(Style::default().fg(color)),
    ])
    .style(Style::default().fg(color))
}

fn summary_row(label: &str, value: Bytes) -> Row<'static> {
    Row::new(vec![
        Cell::from(label.to_string()),
        Cell::from(value.human_iec()).style(FG.fg()),
    ])
    .style(FG.fg())
}

fn accounting_row(label: &str, value: Bytes, total: Bytes) -> Row<'static> {
    Row::new(vec![
        Cell::from(label.to_string()),
        usage_cell(value, total),
    ])
    .style(FG.fg())
}

fn summary_text_row(label: &str, value: &str) -> Row<'static> {
    Row::new(vec![
        Cell::from(label.to_string()),
        Cell::from(value.to_string()),
    ])
}

fn row_process<'a>(
    app: &App,
    processes: &'a Processes,
    row: &FlatProcessRow,
    selected: bool,
    capacity: Bytes,
) -> Row<'a> {
    let node = &processes.tree.nodes[row.index];
    let rollup = app.tree_scope.rollup(node);
    let marker = match row.fold {
        RowFold::Leaf => " ",
        RowFold::Collapsed => "▸",
        RowFold::Expanded => "▾",
    };
    let name = format!("{}{} {}", "  ".repeat(row.depth), marker, node.name);
    Row::new(vec![
        Cell::from(name),
        Cell::from(node.pid.to_string()),
        Cell::from(node.username.as_str()),
        usage_cell(rollup.pss, capacity),
        usage_cell(rollup.uss(), capacity),
        usage_cell(rollup.rss, capacity),
        Cell::from(node.command.as_str()),
    ])
    .style(usage_style_for_role(
        selected,
        rollup.metric(app.metric),
        capacity,
        row.search,
    ))
}

fn row_tmpfs<'a>(
    tmpfs: &'a super::model::Tmpfs,
    row: &FlatTmpfsRow,
    selected: bool,
    capacity: Bytes,
) -> Row<'a> {
    let mount = &tmpfs.mounts[row.mount_index];
    let node = mount.node(row.node_id);
    let marker = match row.fold {
        RowFold::Leaf => " ",
        RowFold::Collapsed => "▸",
        RowFold::Expanded => "▾",
    };
    let name = node
        .path
        .file_name()
        .unwrap_or_else(|| node.path.as_os_str())
        .to_string_lossy();
    let label = format!("{}{} {name}", "  ".repeat(row.depth), marker);
    Row::new(vec![
        Cell::from(label),
        Cell::from(node.kind.label()),
        usage_cell(node.allocated, capacity),
        usage_cell(node.logical, capacity),
        Cell::from(node.path.to_string_lossy()),
    ])
    .style(usage_style_for_role(
        selected,
        node.allocated,
        capacity,
        row.search,
    ))
}

fn row_shared<'a>(
    shared: &'a Shared,
    row: &FlatSharedRow,
    selected: bool,
    capacity: Bytes,
    metric: super::model::Metric,
) -> Row<'a> {
    let object = &shared.objects[row.index];
    Row::new(vec![
        Cell::from(object.kind.label()),
        Cell::from(object.mapped_processes.to_string()),
        usage_cell(object.rollup.pss, capacity),
        usage_cell(object.rollup.rss, capacity),
        Cell::from(object.regions.to_string()),
        Cell::from(object.label.as_str()),
    ])
    .style(usage_style_for_role(
        selected,
        object.rollup.metric(metric),
        capacity,
        row.search,
    ))
}

fn row_object_usage(object: &ObjectUsage, capacity: Bytes) -> Row<'_> {
    Row::new(vec![
        Cell::from(object.kind.label()),
        usage_cell(object.rollup.pss, capacity),
        usage_cell(object.rollup.rss, capacity),
        Cell::from(object.regions.to_string()),
        Cell::from(object.label.as_str()),
    ])
    .style(usage_style(false, object.rollup.pss, capacity))
}

fn tmpfs_detail_lines<'a>(
    mount: &'a TmpfsMount,
    node: &'a super::model::TmpfsNode,
) -> Vec<Line<'a>> {
    let mut lines = vec![
        Line::from(Span::styled(
            node.path.to_string_lossy(),
            ACCENT.fg().add_modifier(Modifier::BOLD),
        )),
        detail_line("Mount", mount.mount_point.to_string_lossy()),
        detail_line("Source", mount.source.as_str()),
        detail_line("Kind", node.kind.label()),
        detail_line("Allocated", node.allocated.human_exact()),
        detail_line("Logical", node.logical.human_exact()),
    ];
    if let Some(limit) = mount.size_limit {
        lines.push(detail_line("Mount size", limit.human_exact()));
        lines.push(detail_line(
            "Utilization",
            format!("{:.1}%", node.allocated.pct_of(limit)),
        ));
    }
    lines
}

fn search_summary_lines(app: &App, capacity: Bytes) -> Vec<Line<'_>> {
    let Some(summary) = app.search_summary() else {
        return Vec::new();
    };
    let pattern = app.search_pattern().unwrap_or_default();
    let mut lines = vec![
        Line::from(Span::styled(
            "regexp matches",
            GOLD.fg().add_modifier(Modifier::BOLD),
        )),
        detail_line("regexp", format!("/{pattern}/")),
        detail_line("matches", summary.matches.to_string()),
        detail_line(summary.lens, summary.total.human_exact()),
        detail_line(
            "pct total",
            format!("{:.2}%", summary.total.pct_of(capacity)),
        ),
    ];
    if matches!(app.tab, super::app::Tab::Processes | super::app::Tab::Tmpfs) {
        lines.push(detail_line("mode", app.search_scope_label()));
    }
    lines.push(Line::from(""));
    lines
}

fn detail_line<'a>(label: &str, value: impl Into<Cow<'a, str>>) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:>12} "), MUTED.fg()),
        Span::styled(value.into(), FG.fg()),
    ])
}

fn header_row<const N: usize>(values: [&'static str; N]) -> Row<'static> {
    Row::new(values.into_iter().map(Cell::from)).style(GOLD.fg().add_modifier(Modifier::BOLD))
}

fn usage_style(selected: bool, value: Bytes, total: Bytes) -> Style {
    row_fg_style(selected, usage_color(value, total))
}

fn usage_style_for_role(selected: bool, value: Bytes, total: Bytes, role: SearchRole) -> Style {
    if role.is_context() && !selected {
        MUTED.fg()
    } else {
        usage_style(selected, value, total)
    }
}

fn row_fg_style(selected: bool, fg: Color) -> Style {
    if selected {
        Style::default()
            .fg(fg)
            .bg(CURSOR.color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fg)
    }
}

fn usage_cell(value: Bytes, total: Bytes) -> Cell<'static> {
    Cell::from(value.human_iec()).style(Style::default().fg(usage_color(value, total)))
}

fn usage_color(value: Bytes, total: Bytes) -> Color {
    if value.0 == 0 || total.0 == 0 {
        return Color::Rgb(105, 113, 121);
    }

    let pct = (value.as_f64() / total.as_f64()).clamp(0.0, 1.0);
    if pct <= 0.03 {
        return blend_rgb((105, 113, 121), (246, 248, 250), pct / 0.03);
    }
    blend_rgb((246, 248, 250), (232, 58, 46), (pct - 0.03) / 0.97)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MeminfoTone {
    Neutral,
    Reserve,
    Pressure,
}

impl MeminfoTone {
    fn for_key(key: &str) -> Self {
        match key {
            "MemAvailable" => Self::Reserve,
            "Dirty" | "Writeback" | "WritebackTmp" | "Unevictable" | "Mlocked" | "SUnreclaim"
            | "KernelStack" | "PageTables" | "SecPageTables" | "NFS_Unstable" | "Bounce"
            | "HardwareCorrupted" | "SwapCached" | "Zswap" | "Zswapped" => Self::Pressure,
            _ => Self::Neutral,
        }
    }

    fn color(self, value: Bytes, meminfo: &Meminfo) -> Color {
        match self {
            Self::Neutral => neutral_meminfo_color(value),
            Self::Reserve => reserve_color(value, meminfo.value("MemTotal").unwrap_or(Bytes::ZERO)),
            Self::Pressure => usage_color(value, meminfo.value("MemTotal").unwrap_or(Bytes::ZERO)),
        }
    }
}

fn neutral_meminfo_color(value: Bytes) -> Color {
    if value.0 == 0 {
        MUTED.color()
    } else {
        FG.color()
    }
}

fn reserve_color(value: Bytes, total: Bytes) -> Color {
    if value.0 == 0 || total.0 == 0 {
        return MUTED.color();
    }

    let pct = (value.as_f64() / total.as_f64()).clamp(0.0, 1.0);
    if pct >= 0.10 {
        return FG.color();
    }
    if pct <= 0.03 {
        return HOT.color();
    }
    blend_rgb((232, 58, 46), (246, 248, 250), (pct - 0.03) / 0.07)
}

fn blend_rgb(start: (u8, u8, u8), end: (u8, u8, u8), t: f64) -> Color {
    Color::Rgb(
        blend_channel(start.0, end.0, t),
        blend_channel(start.1, end.1, t),
        blend_channel(start.2, end.2, t),
    )
}

fn blend_channel(start: u8, end: u8, t: f64) -> u8 {
    (f64::from(start) + (f64::from(end) - f64::from(start)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// What help adds to the bindings.
const NOTES: &[&str] = &[
    "Overview estimates a physical-RAM counter partition; /proc/meminfo counters can overlap and \
     omit subsystem ownership.",
    "NVIDIA system pools appear as an independent lens when shrinker debugfs is readable; they are \
     not subtracted because the kernel snapshots can overlap or race.",
    "PSS lenses remain zero for tasks readable only through status RSS; coverage reports the \
     degradation.",
    "Tmpfs uses allocated blocks, which is closer to actual backing than file length.",
    "Tree panes auto-fold subtrees below min(1% RAM, 3% largest non-root subtree).",
    "Shared aggregates mapped objects across visible tasks: tmpfs, memfd, SYSV, files, and \
     anonymous regions.",
];

fn render_kill_confirmation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(confirmation) = app.kill_confirmation() else {
        return;
    };

    let popup = centered_rect(area, 88, 88);
    frame.render_widget(Clear, popup);
    let block = panel("kill", true);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(inner);

    let top = vec![
        Line::from(Span::styled(
            "Send SIGTERM to this process?",
            HOT.fg().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        detail_line("PID", confirmation.target.pid.to_string()),
        detail_line("Name", confirmation.target.name.as_str()),
    ];
    frame.render_widget(Paragraph::new(top).style(FG.fg()), sections[0]);

    frame.render_widget(
        Paragraph::new(confirmation.target.cli())
            .block(
                Block::default()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .title("Full CLI")
                    .style(ACCENT.fg().bg(BG.color())),
            )
            .wrap(Wrap { trim: false })
            .style(PAGE),
        sections[1],
    );

    let mut controls = Vec::new();
    if confirmation.armed() {
        controls.push(Line::from(Span::styled(
            "press y to send SIGTERM",
            HOT.fg().add_modifier(Modifier::BOLD),
        )));
    } else {
        controls.push(detail_line(
            "lockout",
            format!(
                "{} ms before y is accepted",
                confirmation.lock_remaining().as_millis()
            ),
        ));
    }
    controls.push(detail_line("cancel", "Esc or n"));

    frame.render_widget(
        Paragraph::new(controls)
            .wrap(Wrap { trim: false })
            .style(PAGE),
        sections[2],
    );
}

fn render_search_prompt(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(draft) = app.search_draft() else {
        return;
    };

    let popup = centered_rect(area, 76, 22);
    frame.render_widget(Clear, popup);
    let mut lines = vec![
        Line::from(Span::styled(
            "Filter rows by regexp",
            ACCENT.fg().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("/", GOLD.fg()),
            Span::styled(draft.input().to_string(), FG.fg()),
        ]),
        Line::from(""),
        detail_line("accept", "Enter"),
        detail_line("clear", "empty input, then Enter"),
        detail_line("cancel", "Esc"),
    ];
    if let Some(error) = draft.error() {
        lines.push(detail_line("error", error));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("search", true))
            .wrap(Wrap { trim: false })
            .style(FG.fg()),
        popup,
    );
}

fn centered_rect(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_pct) / 2),
            Constraint::Percentage(height_pct),
            Constraint::Percentage((100 - height_pct) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

struct SliceWindow<'a, T> {
    start: usize,
    slice: &'a [T],
}

impl<T> std::ops::Deref for SliceWindow<'_, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.slice
    }
}

fn slice_window<T>(items: &[T], selected: usize, height: usize) -> SliceWindow<'_, T> {
    if items.is_empty() {
        return SliceWindow {
            start: 0,
            slice: items,
        };
    }
    let height = height.max(1);
    let half = height / 2;
    let start = selected
        .saturating_sub(half)
        .min(items.len().saturating_sub(height));
    let end = (start + height).min(items.len());
    SliceWindow {
        start,
        slice: &items[start..end],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meminfo(total: Bytes) -> Meminfo {
        Meminfo {
            entries: vec![MeminfoEntry {
                key: "MemTotal".to_string(),
                value: total,
            }],
        }
    }

    #[test]
    fn overview_meminfo_capacity_and_empty_like_rows_are_not_hot() {
        let meminfo = meminfo(Bytes(100));

        assert_eq!(MeminfoTone::for_key("MemTotal"), MeminfoTone::Neutral);
        assert_eq!(MeminfoTone::for_key("MemFree"), MeminfoTone::Neutral);
        assert_eq!(MeminfoTone::for_key("SwapFree"), MeminfoTone::Neutral);
        assert_eq!(MeminfoTone::for_key("MemAvailable"), MeminfoTone::Reserve);

        assert_eq!(MeminfoTone::Neutral.color(Bytes(100), &meminfo), FG.color());
        assert_eq!(
            MeminfoTone::Neutral.color(Bytes(0), &meminfo),
            MUTED.color()
        );
        assert_eq!(MeminfoTone::Reserve.color(Bytes(40), &meminfo), FG.color());
        assert_eq!(MeminfoTone::Reserve.color(Bytes(2), &meminfo), HOT.color());
    }

    #[test]
    fn overview_meminfo_hot_rows_are_explicit_pressure_counters() {
        let meminfo = meminfo(Bytes(100));

        assert_eq!(MeminfoTone::for_key("Dirty"), MeminfoTone::Pressure);
        assert_eq!(MeminfoTone::for_key("SUnreclaim"), MeminfoTone::Pressure);
        assert_eq!(MeminfoTone::for_key("AnonPages"), MeminfoTone::Neutral);
        assert_eq!(
            MeminfoTone::Pressure.color(Bytes(0), &meminfo),
            usage_color(Bytes(0), Bytes(100))
        );
    }
}
