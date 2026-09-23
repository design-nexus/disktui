mod app;
mod config;
mod fstab;
mod model;
mod msg;
mod op;
mod privilege;
mod theme;
mod udisks;
mod ui;
mod util;

use std::env;
use std::io::{self, IsTerminal};

use anyhow::Context;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use tokio::sync::mpsc::unbounded_channel;

use app::{App, Effect};
use config::Config;
use msg::Msg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = env::args().skip(1);
    let attach = args.next().filter(|arg| arg != "--help" && arg != "-h");
    if env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!("disktui [image-file]\n\nTerminal disk manager. See the README for keys.");
        return Ok(());
    }
    if !io::stdout().is_terminal() {
        anyhow::bail!("disktui runs in a terminal.");
    }

    let found = theme::find_omarchy();
    let config_path = Config::path();
    let config = Config::load(&config_path, found.is_some());
    let omarchy = found.as_ref().map(|(_, theme, _)| theme.clone());
    let custom = theme::load_user_theme();
    let theme = theme::resolve(&config.theme, omarchy.as_ref(), custom.as_ref());
    let mut app = App::new(config, config_path, theme, omarchy, custom, attach);

    let (tx, mut rx) = unbounded_channel();
    if let Some((_, _, dir)) = found {
        let watch_tx = tx.clone();
        theme::spawn_watcher(dir, move || {
            let _ = watch_tx.send(Msg::ThemeReload);
        });
    }

    let conn = match udisks::connect().await {
        Ok(conn) => {
            let watch = conn.clone();
            let watch_tx = tx.clone();
            tokio::spawn(async move { udisks::watch(watch, watch_tx).await });
            let caps_conn = conn.clone();
            let caps_tx = tx.clone();
            tokio::spawn(async move {
                let caps = udisks::load_caps(&caps_conn).await;
                let _ = caps_tx.send(Msg::Caps(caps));
            });
            Some(conn)
        }
        Err(err) => {
            app.error = format!("UDisks is not available ({err}). Network shares still work.");
            None
        }
    };

    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        ratatui::restore();
        hook(info);
    }));
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, conn, &mut rx, tx).await;
    ratatui::restore();
    result
}

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    conn: Option<zbus::Connection>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Msg>,
    tx: tokio::sync::mpsc::UnboundedSender<Msg>,
) -> anyhow::Result<()> {
    let mut events = EventStream::new();
    loop {
        terminal.draw(|frame| ui::draw(frame, app)).context("draw")?;
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(80)), if app.busy => {
                app.busy_tick = app.busy_tick.wrapping_add(1);
            }
            event = events.next() => {
                match event {
                    Some(Ok(Event::Key(key))) => {
                        if key.kind == KeyEventKind::Release {
                            continue;
                        }
                        app.on_key(key);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => {}
                    None => app.quit = true,
                }
            }
            msg = rx.recv() => {
                if let Some(msg) = msg {
                    app.on_msg(msg);
                }
            }
        }
        match app.take_effect() {
            Effect::None => {}
            Effect::Call(op) => {
                let Some(conn) = conn.clone() else {
                    app.error = "UDisks is not available.".into();
                    app.busy = false;
                    continue;
                };
                let tx = tx.clone();
                tokio::spawn(async move { udisks::execute(&conn, op, tx).await });
            }
            Effect::Sudo(job) => {
                let result = privilege::run(&job);
                privilege::resume(terminal).context("restore terminal")?;
                match result {
                    Ok(_) => {
                        app.error.clear();
                        app.info = match &job {
                            privilege::SudoJob::Mount(_) => "Mounted.".into(),
                            privilege::SudoJob::Umount(_) => "Unmounted.".into(),
                            privilege::SudoJob::ApplyFstab { .. } => "fstab updated.".into(),
                        };
                    }
                    Err(err) => app.error = err,
                }
                app.reload_fstab();
            }
        }
        if app.quit {
            break;
        }
    }
    Ok(())
}
