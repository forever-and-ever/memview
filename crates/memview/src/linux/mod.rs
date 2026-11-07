mod activity;
mod app;
mod model;
mod nav;
mod probe;
mod search;
mod state;
mod ui;

use activity::Activity;
use app::{App, spawn_worker};
use clap::Parser;
use color_eyre::eyre::{Result, ensure};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use libshimmer::TICK;
use state::UiStateStore;
use std::io::{self, IsTerminal};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tui_eternal::{Reports, Screen};

const FOCUSED_IDLE_POLL: Duration = Duration::from_millis(500);
const BACKGROUND_IDLE_POLL: Duration = Duration::from_secs(1);
/// Animated content, such as the activity shimmer, redraws every shimmer tick.
const ANIMATED_REDRAW: Duration = TICK;

#[derive(Debug, Parser)]
#[command(author, version, about = "ncdu-like RAM accounting for Linux /proc")]
struct Cli {
    /// Minimum automatic refresh delay; expensive scans extend it to limit work.
    #[arg(long, default_value_t = 5000)]
    refresh_ms: u64,
}

pub type MainResult = Result<()>;

pub fn run() -> MainResult {
    color_eyre::install()?;
    let cli = Cli::parse();
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "memview requires an interactive terminal on stdin and stdout"
    );
    let termination = TerminationFlag::install()?;
    let (commands, events) = spawn_worker(Duration::from_millis(cli.refresh_ms));
    let mut state_store = UiStateStore::discover();
    let mut screen = Screen::open(Reports {
        mouse: true,
        focus: true,
        paste: false,
    })?;
    let mut app = App::new(state_store.restored());
    app.last_error = state_store.take_warning();
    if let Some(warning) = state_store.sync(app.ui_state()) {
        app.last_error = Some(warning);
    }
    app.set_terminal_height(screen.size()?.height);
    app.start_visible_work(&commands);
    let mut activity = Activity::summon(Instant::now())?;
    let mut dirty = true;
    let mut next_animated_redraw = Instant::now();

    let termination_signal = loop {
        if let Some(signal) = termination.pending() {
            break Some(signal);
        }
        dirty |= app.pause_if_idle(&commands);
        while let Ok(event) = events.try_recv() {
            app.apply_worker_event(event, &commands);
            dirty = true;
        }

        let now = Instant::now();
        let redraw_animation = app.needs_periodic_redraw() && now >= next_animated_redraw;
        if app.is_focused() && (dirty || redraw_animation) {
            draw(&mut screen, &app, &mut activity)?;
            dirty = false;
            next_animated_redraw = Instant::now() + ANIMATED_REDRAW;
        }

        let input_ready = match event::poll(poll_timeout(&app, next_animated_redraw)) {
            Ok(input_ready) => input_ready,
            Err(_) if termination.pending().is_some() => break termination.pending(),
            Err(error) => return Err(error.into()),
        };
        if input_ready {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    app.note_interaction(&commands);
                    let quit = app.handle_key(key, &commands);
                    if let Some(warning) = state_store.sync(app.ui_state()) {
                        app.last_error = Some(warning);
                    }
                    if quit {
                        break None;
                    }
                    dirty = true;
                }
                Event::Mouse(mouse) => {
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) {
                        app.note_interaction(&commands);
                        dirty = true;
                    }
                    if app.handle_mouse(mouse, &commands) {
                        dirty = true;
                    }
                }
                Event::Resize(_, height) => {
                    app.set_terminal_height(height);
                    dirty = true;
                }
                Event::FocusGained => {
                    app.set_focused(true, &commands);
                    dirty = true;
                }
                Event::FocusLost => {
                    app.set_focused(false, &commands);
                    // The terminal may remain visible after losing focus. Paint
                    // the pause reason once, then leave its contents untouched.
                    draw(&mut screen, &app, &mut activity)?;
                    dirty = false;
                }
                Event::Paste(_) => {}
            }
        }
    };

    commands.shutdown();
    drop(screen);
    if let Some(signal) = termination_signal {
        std::process::exit(128 + signal);
    }
    Ok(())
}

struct TerminationFlag(Arc<AtomicUsize>);

impl TerminationFlag {
    fn install() -> Result<Self> {
        use signal_hook::consts::signal::{SIGHUP, SIGTERM};

        let pending = Arc::new(AtomicUsize::new(0));
        let _hangup = signal_hook::flag::register_usize(
            SIGHUP,
            Arc::clone(&pending),
            usize::try_from(SIGHUP)?,
        )?;
        let _terminate = signal_hook::flag::register_usize(
            SIGTERM,
            Arc::clone(&pending),
            usize::try_from(SIGTERM)?,
        )?;
        Ok(Self(pending))
    }

    fn pending(&self) -> Option<i32> {
        i32::try_from(self.0.load(Ordering::SeqCst))
            .ok()
            .filter(|signal| *signal != 0)
    }
}

/// Advances the activity shimmer to now and draws a frame.
fn draw(screen: &mut Screen, app: &App, activity: &mut Activity) -> Result<()> {
    activity.follow(app, Instant::now());
    screen.draw(|frame| ui::render(frame, app, activity))?;
    Ok(())
}

fn poll_timeout(app: &App, next_animated_redraw: Instant) -> Duration {
    if !app.is_focused() {
        return BACKGROUND_IDLE_POLL;
    }
    if app.needs_periodic_redraw() {
        FOCUSED_IDLE_POLL.min(next_animated_redraw.saturating_duration_since(Instant::now()))
    } else {
        FOCUSED_IDLE_POLL
    }
}
