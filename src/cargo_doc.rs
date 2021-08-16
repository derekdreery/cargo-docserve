//! Things that run `cargo doc`.
use anyhow::{format_err, Error};
use crossbeam_channel::unbounded;
use notify::{event::Event, Error as NError, RecursiveMode, Watcher};
use qu::ick_use::*;
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use crate::Config;

enum NotifyMsg {
    Event(Result<Event, NError>),
    Shutdown,
}

enum CargoMsg {
    Run,
    Shutdown,
}

/// Run `cargo doc` once.
pub(crate) fn run(config: &Config) -> Result<(), Error> {
    let mut cmd = Command::new("cargo");
    cmd.arg("doc");
    if let Some(m) = &config.manifest {
        cmd.args(&["--manifest-path", m]);
    }
    cmd.args(config.cargo_args.iter().map(|s| s.as_str()))
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    log::debug!("running `{:?}`", cmd);
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format_err!(
            "cargo doc failed with error code {:?}",
            status.code()
        ))
    }
}

/// Spawn a thread to run `cargo doc` when a file change is detected.
///
/// Call the supplied callback to shutdown this thread.
pub(crate) fn watch(config: Arc<Config>) -> Result<impl FnOnce()> {
    let (tx, rx) = unbounded();
    let (run_tx, run_rx) = unbounded();

    // setup notify
    let mut watcher = notify::RecommendedWatcher::new({
        let tx = tx.clone();
        move |evt| {
            let _ = tx.send(NotifyMsg::Event(evt));
        }
    })?;
    watcher
        .watch(
            config.metadata.workspace_root.as_std_path(),
            RecursiveMode::Recursive,
        )
        .context(format!(
            "error watching \"{}\"",
            config.metadata.workspace_root
        ))?;
    for extra in config.watch.as_ref().unwrap().iter() {
        watcher
            .watch(Path::new(extra), RecursiveMode::Recursive)
            .context(format!("error watching \"{}\"", extra))?;
    }
    watcher
        .unwatch(config.metadata.target_directory.as_std_path())
        .context(format!(
            "error unwatching \"{}\"",
            config.metadata.target_directory
        ))?;

    // notify thread
    let notify_thread = thread::spawn({
        let run_tx = run_tx.clone();
        move || {
            // move watcher here so it lives as long as we are handling messages.
            let _watcher = watcher;
            loop {
                match rx.recv() {
                    Ok(NotifyMsg::Event(Ok(evt))) => {
                        log::trace!("receive notify event {:?}", evt);
                        let _ = run_tx.send(CargoMsg::Run);
                    }
                    Ok(NotifyMsg::Event(Err(err))) => {
                        log::error!("`notify` reported error: {:#}", err);
                    }
                    // main thread is going away
                    Ok(NotifyMsg::Shutdown) | Err(_) => break,
                }
            }
        }
    });

    // cargo doc thread
    let cargo_thread = thread::spawn({
        move || 'main: loop {
            match run_rx.recv() {
                Ok(CargoMsg::Run) => {
                    // debounce a little
                    thread::sleep(Duration::from_millis(10));
                    // drain the channel
                    while let Ok(msg) = run_rx.try_recv() {
                        if matches!(msg, CargoMsg::Shutdown) {
                            break 'main;
                        }
                    }
                    // rebuild docs
                    if let Err(e) = run(&*config) {
                        log::error!("error running `cargo doc`: {}", e);
                    }
                }
                Ok(CargoMsg::Shutdown) | Err(_) => break,
            }
        }
    });

    Ok(move || {
        // ignore errors since that means the thread is gone anyway.
        let _ = tx.send(NotifyMsg::Shutdown);
        let _ = run_tx.send(CargoMsg::Shutdown);
        notify_thread.join().unwrap();
        cargo_thread.join().unwrap();
    })
}
