//! Bridge between GPUI's executor and the tokio runtime that `reqwest` needs.

use std::future::Future;

use gpui::{App, Global};

/// The network runtime, stored as a GPUI global.
struct NetRuntime(tokio::runtime::Runtime);

impl Global for NetRuntime {}

/// Starts the network runtime. Call once at startup.
pub fn init(cx: &mut App) -> std::io::Result<()> {
  let runtime = tokio::runtime::Builder::new_multi_thread()
    .worker_threads(2)
    .thread_name("peeby-net")
    .enable_all()
    .build()?;
  cx.set_global(NetRuntime(runtime));
  Ok(())
}

/// The network runtime, for what spawns its own tasks on it (a tunnel, a local proxy).
pub fn handle(cx: &App) -> tokio::runtime::Handle {
  cx.global::<NetRuntime>().0.handle().clone()
}

/// Runs `future` on the network runtime. The returned future can be awaited from GPUI tasks
/// (`cx.spawn`); it resolves to `None` only if the runtime shut down before finishing.
pub fn spawn<T, F>(cx: &App, future: F) -> impl Future<Output = Option<T>> + use<T, F>
where
  T: Send + 'static,
  F: Future<Output = T> + Send + 'static,
{
  let (tx, rx) = tokio::sync::oneshot::channel();
  cx.global::<NetRuntime>().0.spawn(async move {
    let _ = tx.send(future.await);
  });
  async move { rx.await.ok() }
}
