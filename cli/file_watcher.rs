// Copyright 2018-2020 the Deno authors. All rights reserved. MIT license.

use crate::colors;
use deno_core::error::AnyError;
use deno_core::futures::stream::StreamExt;
use deno_core::futures::Future;
use notify::event::Event as NotifyEvent;
use notify::event::EventKind;
use notify::Config;
use notify::Error as NotifyError;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Instant;
use tokio::select;
use tokio::sync::{mpsc, mpsc::Receiver};

/// Time without update required to pass to assume after which fluctuations are over
const DEBOUNCE_TIME_MS: usize = 200;

// TODO(bartlomieju): rename
type WatchFuture = Pin<Box<dyn Future<Output = Result<(), AnyError>>>>;

struct Debounce {
  last: Instant,
  duration_ms: usize,
}

impl Debounce {
  /// Creates debounce instance
  /// that takes time in ms after which fluctuations are recognized to be over
  fn new(duration_ms: usize) -> Self {
    let last = Instant::now();
    Self { last, duration_ms }
  }

  /// Check if fluctuations has passed
  fn fluct_is_over(&mut self) -> bool {
    let waiting_fluct_is_over =
      self.duration_ms <= self.last.elapsed().as_millis() as usize;
    self.last = Instant::now();
    waiting_fluct_is_over
  }
}

async fn error_handler(watch_future: WatchFuture) {
  let result = watch_future.await;
  if let Err(err) = result {
    let msg = format!("{}: {}", colors::red_bold("error"), err.to_string(),);
    eprintln!("{}", msg);
  }
}

pub async fn watch_func<F>(
  paths: &[PathBuf],
  closure: F,
) -> Result<(), AnyError>
where
  F: Fn() -> WatchFuture,
{
  let mut debounce = Debounce::new(DEBOUNCE_TIME_MS);
  let (_watcher, receiver) = new_watcher(paths)?;
  let receiver = Mutex::new(receiver);
  loop {
    let func = error_handler(closure());
    let mut is_file_changed = false;
    select! {
      _ = wait_for_file_change(&receiver, &mut debounce) => {
          is_file_changed = true;
          info!(
            "{} File change detected! Restarting!",
            colors::intense_blue("Watcher")
          );
        },
      _ = func => { },
    };
    if !is_file_changed {
      info!(
        "{} Process terminated! Restarting on file change...",
        colors::intense_blue("Watcher")
      );
      wait_for_file_change(&receiver, &mut debounce).await?;
      info!(
        "{} File change detected! Restarting!",
        colors::intense_blue("Watcher")
      );
    }
  }
}

async fn wait_for_file_change(
  receiver: &Mutex<Receiver<Result<NotifyEvent, AnyError>>>,
  debounce: &mut Debounce,
) -> Result<(), AnyError> {
  while let Some(result) = receiver.lock().unwrap().next().await {
    let event = result?;
    if debounce.fluct_is_over() {
      match event.kind {
        EventKind::Create(_) => break,
        EventKind::Modify(_) => break,
        EventKind::Remove(_) => break,
        _ => continue,
      }
    }
  }
  Ok(())
}

fn new_watcher(
  paths: &[PathBuf],
) -> Result<
  (RecommendedWatcher, Receiver<Result<NotifyEvent, AnyError>>),
  AnyError,
> {
  let (sender, receiver) = mpsc::channel::<Result<NotifyEvent, AnyError>>(16);
  let sender = Mutex::new(sender);

  let mut watcher: RecommendedWatcher =
    Watcher::new_immediate(move |res: Result<NotifyEvent, NotifyError>| {
      let res2 = res.map_err(AnyError::from);
      let mut sender = sender.lock().unwrap();
      // Ignore result, if send failed it means that watcher was already closed,
      // but not all messages have been flushed.
      let _ = sender.try_send(res2);
    })?;

  watcher.configure(Config::PreciseEvents(true)).unwrap();

  for path in paths {
    watcher.watch(path, RecursiveMode::NonRecursive)?;
  }

  Ok((watcher, receiver))
}

#[test]
fn debounce_test() {
  let mut debounce = Debounce::new(50);
  assert!(!debounce.fluct_is_over());
  std::thread::sleep(std::time::Duration::from_millis(0));
  assert!(!debounce.fluct_is_over());
  std::thread::sleep(std::time::Duration::from_millis(10));
  assert!(!debounce.fluct_is_over());
  std::thread::sleep(std::time::Duration::from_millis(49));
  assert!(!debounce.fluct_is_over());
  std::thread::sleep(std::time::Duration::from_millis(50));
  assert!(debounce.fluct_is_over());
}
