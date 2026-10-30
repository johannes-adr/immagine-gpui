use futures_channel::oneshot;

/// Runs blocking work on its own thread and resolves with the result, so the GPUI executor never blocks.
pub async fn run<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.await.unwrap_or_else(|_| Err("worker thread ended".into()))
}
