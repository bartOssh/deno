// Copyright 2018-2020 the Deno authors. All rights reserved. MIT license.

use crate::colors;
use core::task::{Context, Poll};
use deno_core::error::AnyError;
use deno_core::futures::stream::{Stream, StreamExt};
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
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, mpsc::Receiver};

/// Time without update required to pass to assume after which fluctuations are over
const DEBOUNCE_TIME_MS: Duration = Duration::from_millis(200);

// TODO(bartlomieju): rename
type WatchFuture = Pin<Box<dyn Future<Output = Result<(), AnyError>>>>;

// TODO(bartossh): make generic and move to unique mod
struct Debounce {
  rx: Receiver<Result<NotifyEvent, AnyError>>,
  debounce_time: Duration,
  last_event: NotifyEvent,
}

impl Debounce {
  fn new(
    rx: Receiver<Result<NotifyEvent, AnyError>>,
    debounce_time: Duration,
  ) -> Self {
    Self {
      rx,
      debounce_time,
      last_event: Default::default(),
    }
  }
}

impl Stream for Debounce {
  type Item = NotifyEvent;

  fn poll_next(
    self: Pin<&mut Self>,
    _cx: &mut Context,
  ) -> Poll<Option<Self::Item>> {
    let mut _self = self.get_mut();
    let mut timeout = Instant::now();
    let mut recv = false;
    loop {
      if let Ok(result) = _self.rx.try_recv() {
        if let Ok(event) = result {
          if event == _self.last_event {
            timeout = Instant::now();
          }
          _self.last_event = event;
          recv = true;
        }
      }
      if recv && timeout.elapsed() >= _self.debounce_time {
        break;
      }
    }
    Poll::Ready(Some(_self.last_event.clone()))
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
  let (_watcher, receiver) = new_watcher(paths)?;
  let debounce = Mutex::new(Debounce::new(receiver, DEBOUNCE_TIME_MS));
  loop {
    let func = error_handler(closure());
    func.await;
    info!(
      "{} Process terminated! Restarting on file change...",
      colors::intense_blue("Watcher")
    );
    wait_for_file_change(&debounce).await?;
    info!(
      "{} File change detected! Restarting!",
      colors::intense_blue("Watcher")
    );
  }
}

async fn wait_for_file_change(
  debounce: &Mutex<Debounce>,
) -> Result<(), AnyError> {
  while let Some(event) = debounce.lock().unwrap().next().await {
    match event.kind {
      EventKind::Create(_) => break,
      EventKind::Modify(_) => break,
      EventKind::Remove(_) => break,
      _ => continue,
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
