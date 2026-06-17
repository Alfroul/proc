pub mod alert_badge;
pub mod app_group_view;
pub mod detail_view;
pub mod docker_panel;
pub mod help_panel;
pub mod layout;
pub mod monitor_panel;
pub mod port_table;
pub mod process_table;
pub mod process_tree;
pub mod replay_panel;
pub mod right_panel;
pub mod security_badge;
pub mod sidebar;
pub mod theme;
pub mod usb_panel;

use std::io::Stdout;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph};

use crate::app::{App, ReplaySpeed};
use crate::error::Result;
use crate::record::vt100::{VtFrameWidget, VtPlayer, VtRecorder};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

#[must_use]
pub fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_width = r.width * percent_x / 100;
    let x = r.width.saturating_sub(popup_width) / 2;
    let y = r.height.saturating_sub(height) / 2;
    Rect::new(
        r.x + x,
        r.y + y,
        popup_width.min(r.width),
        height.min(r.height),
    )
}

pub fn setup_terminal() -> Result<Tui> {
    terminal::enable_raw_mode()?;
    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    crossterm::execute!(terminal.backend_mut(), EnterAlternateScreen)?;

    // Install panic hook to restore terminal before printing panic info
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen);
        prev_hook(info);
    }));

    Ok(terminal)
}

pub fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    Ok(())
}

pub fn run_app(terminal: &mut Tui, app: &mut App) -> Result<()> {
    let mut vt_recorder: Option<VtRecorder> = None;
    let frame_time = Duration::from_millis(50);

    while !app.should_quit {
        if crate::shutdown::requested() {
            app.should_quit = true;
        }

        let start = Instant::now();

        handle_events(app)?;
        let data_changed = app.tick();

        // Manage VT100 recorder lifecycle
        if app.recording_wanted() && vt_recorder.is_none() {
            let size = terminal
                .size()
                .unwrap_or(ratatui::layout::Size::new(80, 24));
            let path = default_vt_recording_path();
            match VtRecorder::start(path, size.width, size.height) {
                Ok(rec) => {
                    app.set_status("VT100 录制已开始，按 Shift+R 停止".to_string());
                    vt_recorder = Some(rec);
                }
                Err(e) => {
                    app.set_status(format!("录制启动失败: {}", e));
                    app.set_recording_wanted(false);
                }
            }
        } else if !app.recording_wanted()
            && vt_recorder.is_some()
            && let Some(rec) = vt_recorder.take()
        {
            match rec.stop() {
                Ok(path) => app.set_status(format!("录制已保存: {}", path.display())),
                Err(e) => app.set_status(format!("录制保存失败: {}", e)),
            }
        }

        if data_changed || app.pending_redraw {
            app.pending_redraw = false;
            let completed = terminal.draw(|f| layout::draw(f, app))?;

            if let Some(ref mut rec) = vt_recorder {
                rec.try_capture(completed.buffer, completed.area);
            }
        }

        // Update recording elapsed in App for sidebar display
        if let Some(ref rec) = vt_recorder {
            app.set_recording_elapsed(rec.elapsed_secs());
        }

        let elapsed = start.elapsed();
        if let Some(remain) = frame_time.checked_sub(elapsed) {
            std::thread::sleep(remain);
        }
    }

    // Ensure recorder is flushed even on Ctrl+C — its stop() joins the writer
    // thread and flushes the underlying file.
    if let Some(rec) = vt_recorder.take() {
        rec.stop().ok();
    }

    app.shutdown();

    Ok(())
}

fn handle_events(app: &mut App) -> Result<()> {
    let mut count = 0;
    while event::poll(Duration::from_millis(0))? && count < 10 {
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
        count += 1;
    }
    Ok(())
}

// ── VT100 Replay ──

struct ReplayState {
    current: usize,
    total: usize,
    speed: ReplaySpeed,
    playing: bool,
    quit: bool,
    last_tick: Instant,
}

impl Default for ReplayState {
    fn default() -> Self {
        Self {
            current: 0,
            total: 0,
            speed: ReplaySpeed::Normal,
            playing: false,
            quit: false,
            last_tick: Instant::now(),
        }
    }
}

pub fn run_vt_replay(terminal: &mut Tui, player: VtPlayer) -> Result<()> {
    let mut state = ReplayState {
        total: player.total_frames(),
        ..ReplayState::default()
    };
    let frame_time = Duration::from_millis(50);

    while !state.quit {
        if crate::shutdown::requested() {
            state.quit = true;
        }

        // Handle key events
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                handle_vt_replay_key(&mut state, &player, key);
            }
        }

        // Auto-advance
        if state.playing && state.total > 0 {
            let interval = replay_interval(&player, &state);
            if state.last_tick.elapsed() >= interval {
                if state.current + 1 < state.total {
                    state.current += 1;
                    state.last_tick = Instant::now();
                } else {
                    state.playing = false;
                }
            }
        }

        // Render
        terminal.draw(|f| {
            let area = f.area();

            // Render recorded frame content (leave bottom 5 rows for timeline)
            let timeline_h: u16 = 5;
            let main_h = area.height.saturating_sub(timeline_h);
            let main_area = Rect::new(0, 0, area.width, main_h);
            let timeline_area = Rect::new(0, main_h, area.width, timeline_h);

            if let Some(frame) = player.frame_at(state.current) {
                let widget = VtFrameWidget::new(frame);
                f.render_widget(widget, main_area);
            } else {
                let p = Paragraph::new("No frames");
                f.render_widget(p, main_area);
            }

            draw_vt_timeline(f, timeline_area, &state, &player);
        })?;

        std::thread::sleep(frame_time);
    }

    Ok(())
}

fn handle_vt_replay_key(state: &mut ReplayState, _player: &VtPlayer, key: KeyEvent) {
    match key.code {
        crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => {
            state.quit = true;
        }
        crossterm::event::KeyCode::Char(' ') if state.total > 0 => {
            state.playing = !state.playing;
            state.last_tick = Instant::now();
        }
        crossterm::event::KeyCode::Left => {
            let step = if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT)
            {
                10
            } else {
                1
            };
            state.current = state.current.saturating_sub(step);
            state.playing = false;
        }
        crossterm::event::KeyCode::Right => {
            let step = if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT)
            {
                10
            } else {
                1
            };
            state.current = (state.current + step).min(state.total.saturating_sub(1));
            state.playing = false;
        }
        crossterm::event::KeyCode::Char('+') | crossterm::event::KeyCode::Char('=') => {
            state.speed = match state.speed {
                ReplaySpeed::Half => ReplaySpeed::Normal,
                ReplaySpeed::Normal => ReplaySpeed::Double,
                ReplaySpeed::Double => ReplaySpeed::Quad,
                ReplaySpeed::Quad => ReplaySpeed::Quad,
            };
        }
        crossterm::event::KeyCode::Char('-') => {
            state.speed = match state.speed {
                ReplaySpeed::Half => ReplaySpeed::Half,
                ReplaySpeed::Normal => ReplaySpeed::Half,
                ReplaySpeed::Double => ReplaySpeed::Normal,
                ReplaySpeed::Quad => ReplaySpeed::Double,
            };
        }
        crossterm::event::KeyCode::Home => {
            state.current = 0;
            state.playing = false;
        }
        crossterm::event::KeyCode::End => {
            state.current = state.total.saturating_sub(1);
            state.playing = false;
        }
        _ => {}
    }
}

fn replay_interval(player: &VtPlayer, state: &ReplayState) -> Duration {
    if state.current + 1 >= state.total {
        return Duration::from_secs(1);
    }
    let cur = player.frame_at(state.current);
    let next = player.frame_at(state.current + 1);
    let real_ms = match (cur, next) {
        (Some(c), Some(n)) => n.timestamp_ms.saturating_sub(c.timestamp_ms),
        _ => 1000,
    };
    let adjusted = (real_ms as f64 / state.speed.as_f32() as f64) as u64;
    Duration::from_millis(adjusted.max(16))
}

fn draw_vt_timeline(f: &mut ratatui::Frame, area: Rect, state: &ReplayState, player: &VtPlayer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" VT100 回放 ")
        .style(crate::tui::theme::style_normal());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [info_area, gauge_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Length(1),
    ])
    .areas(inner);

    let icon = if state.playing {
        "\u{25B6}"
    } else {
        "\u{23F8}"
    };
    let speed_label = match state.speed {
        ReplaySpeed::Half => "0.5x",
        ReplaySpeed::Normal => "1x",
        ReplaySpeed::Double => "2x",
        ReplaySpeed::Quad => "4x",
    };

    let (start_ms, end_ms) = player.time_range_ms();
    let current_ms = player
        .frame_at(state.current)
        .map(|f| f.timestamp_ms)
        .unwrap_or(start_ms);
    let end_str = format_timestamp(end_ms);
    let current_str = format_timestamp(current_ms);
    let duration_ms = end_ms.saturating_sub(start_ms);
    let duration_str = format_duration(duration_ms / 1000);

    let info_line = Line::from(vec![
        Span::styled(format!(" {} ", icon), crate::tui::theme::style_selected()),
        Span::styled(
            format!("{} ", speed_label),
            crate::tui::theme::style_muted(),
        ),
        Span::styled(
            format!("{} / {} ", current_str, end_str),
            crate::tui::theme::style_normal(),
        ),
        Span::styled(
            format!("({})", duration_str),
            crate::tui::theme::style_muted(),
        ),
        Span::styled(
            format!("  帧 {}/{}", state.current + 1, state.total),
            crate::tui::theme::style_muted(),
        ),
    ]);
    f.render_widget(Paragraph::new(info_line), info_area);

    let progress = if state.total > 1 {
        state.current as f64 / (state.total - 1) as f64
    } else {
        0.0
    };
    let gauge = Gauge::default()
        .gauge_style(crate::tui::theme::style_selected())
        .ratio(progress.min(1.0));
    f.render_widget(gauge, gauge_area);
}

fn format_timestamp(ms: u64) -> String {
    let secs = ms / 1000;
    let offset_secs = crate::local_offset_hours() * 3600;
    let local = secs + offset_secs as u64;
    let (_, month, day) = crate::epoch_secs_to_ymd(local);
    let h = ((local / 3600) % 24) as u8;
    let m = ((local / 60) % 60) as u8;
    let s = (local % 60) as u8;
    format!("{:02}-{:02} {:02}:{:02}:{:02}", month, day, h, m, s)
}

fn format_duration(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{:02}:{:02}", m, s)
}

fn default_vt_recording_path() -> std::path::PathBuf {
    let dir = crate::dirs_config_dir().join("recordings");
    std::fs::create_dir_all(&dir).ok();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    dir.join(format!("recording_{}.prec", ts))
}
