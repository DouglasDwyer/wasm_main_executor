#![deny(warnings)]

//! ### Run futures on the main browser thread
//! 
//! Certain tasks, like creating an `AudioContext` or `RtcPeerConnection`, can only be performed on the main browser thread.
//! `wasm_main_executor` provides an easy way to send futures to the main browser thread from any context. This allows
//! web workers to spawn main-threaded tasks and await their completion, and facilitates the implementation of cross-thread
//! polyfills/shims.
//! 
//! ## Usage
//! 
//! The following is a simple example of executor usage:
//! 
//! ```rust
//! async fn test_future() -> i32 {
//!     // Do some async or main-threaded work...
//!     2
//! }
//! 
//! // Start the executor. This must be called from the main browser thread.
//! wasm_main_executor::initialize().unwrap();
//! 
//! // Futures may be spawned on background threads using the executor.
//! // The future runs on the main thread.
//! let fut = wasm_main_executor::spawn(test_future());
//! 
//! // A future is returned which may be awaited on the background thread.
//! assert_eq!(2, futures::executor::block_on(fut));
//! ```

use arc_swap::*;
use dummy_waker::*;
use futures::*;
use futures::channel::mpsc::*;
use gloo_timers::callback::*;
use once_cell::sync::*;
use std::collections::*;
use std::future::*;
use std::pin::*;
use std::sync::*;
use std::task::*;
use thiserror::*;
use web_sys::*;

/// Describes an error that occurred while starting the executor.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ExecutorInitializationError {
    /// Multiple attempts were made to start the executor.
    #[error("The executor was already started.")]
    AlreadyStarted(),
    /// The executor was not started on the main thread.
    #[error("The executor was not started on the main thread.")]
    IncorrectThread(),
}

/// Describes a sendable function which generates a future that is not necessarily send.
type SendableFuture = dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send;

/// The queue to which all threads submit newly-spawned tasks.
static TASK_QUEUE: OnceCell<UnboundedSender<Box<SendableFuture>>> = OnceCell::new();

/// Initializes the main thread executor. This function must be called from the main thread
/// before spawning any futures.
pub fn initialize() -> Result<(), ExecutorInitializationError> {
    let mut executor = MainExecutor::new()?;
    Interval::new(0, move || executor.poll()).forget();
    Ok(())
}

/// Spawns a new task for the main thread executor, providing a handle
/// which may be used to await the task result.
pub fn spawn<F: 'static + IntoFuture + Send>(f: F) -> impl Future<Output = F::Output> + Send + Sync where F::Output: Send {
    let sync = FutureSynchronization::default();
    
    let result_ref = sync.sender.clone();
    let waker_ref = sync.waker.clone();

    TASK_QUEUE.get().expect("Main executor was not initialized.").unbounded_send(Box::new(move || Box::pin(async move {
        drop(result_ref.unbounded_send(f.into_future().await));
        if let Some(waker) = &*waker_ref.load() {
            waker.wake_by_ref();
        }
    }))).expect("Could not spawn new task.");

    sync
}

/// Polls futures spawned on the main thread to completion.
struct MainExecutor {
    futures: VecDeque<Pin<Box<dyn Future<Output = ()>>>>,
    receiver: UnboundedReceiver<Box<SendableFuture>>
}

impl MainExecutor {
    /// Creates a new main executor instance and initializes the task queue.
    pub fn new() -> Result<Self, ExecutorInitializationError> {
        if window().is_some() {
            let (send, receiver) = unbounded();
            TASK_QUEUE.set(send).map_err(|_| ExecutorInitializationError::AlreadyStarted())?;

            Ok(Self {
                futures: VecDeque::new(),
                receiver
            })
        }
        else {
            Err(ExecutorInitializationError::IncorrectThread())
        }
    }

    /// Polls all currently-executing futures for this interval.
    pub fn poll(&mut self) {
        self.add_new_futures();
        
        let waker = dummy_waker();
        let mut cx = Context::from_waker(&waker);
        let to_poll = self.futures.len();
        for _ in 0..to_poll {
            let mut fut = self.futures.pop_front().expect("Could not take future from futures queue.");
            match fut.poll_unpin(&mut cx) {
                Poll::Pending => self.futures.push_back(fut),
                Poll::Ready(()) => {}
            }
        }
    }

    /// Adds all newly-spawned futures to the threadpool.
    fn add_new_futures(&mut self) {
        while let Ok(Some(fut)) = self.receiver.try_next() {
            self.futures.push_back(fut());
        }
    }
}

/// Provides the ability to wait on a future from another thread.
struct FutureSynchronization<T> {
    /// A sender which may be used to store the final result of the future.
    pub sender: UnboundedSender<T>,
    /// The receiver which will contain the ultimate result of the future.
    pub receiver: UnboundedReceiver<T>,
    /// The waker that should be alerted when a result is available.
    pub waker: Arc<ArcSwapOption<Waker>>
}

impl<T> Default for FutureSynchronization<T> {
    fn default() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver, waker: Default::default() }
    }
}

impl<T> Future for FutureSynchronization<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.waker.store(Some(Arc::new(cx.waker().clone())));
        if let Ok(Some(res)) = self.receiver.try_next() {
            Poll::Ready(res)
        }
        else {
            Poll::Pending
        }
    }
}