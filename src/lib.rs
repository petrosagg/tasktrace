//! Logical 'stack' traces of async functions.
//!
//! This crate gives you the ability to capture a stacktrace of a running async task.
//!
//! ## Usage
//!
//! To use, wrap the future you're interested in (usually the top level one) with
//! `tasktrace::trace` and use the returned handle to request a backtrace at any time.
//!
//! ```rust
//! #[tokio::main]
//! async fn main() {
//!     let (foo_fut, trace_handle) = tasktrace::traced(foo());
//!     tokio::spawn(foo_fut);
//!
//!     println!("{}", trace_handle.backtrace().await.unwrap());
//! }
//!
//! async fn pending() {
//!     let mut waker = None;
//!     std::future::poll_fn(|cx| {
//!         // Pretend to stash the waker for some future time like a real leaf Future would do.
//!         // This is what records the stacktrace.
//!         waker = Some(cx.waker().clone());
//!         std::task::Poll::Pending
//!     }).await
//! }
//!
//! async fn foo() {
//!     bar().await;
//! }
//!
//! async fn bar() {
//!     tokio::join!(fiz(), buz());
//! }
//!
//! async fn fiz() {
//!     pending().await;
//! }
//!
//! async fn buz() {
//!     baz().await;
//! }
//!
//! async fn baz() {
//!     pending().await;
//! }
//! ```
//!
//! This example program will print out something along the lines of:
//!
//! ```text
//! ╼ <tasktrace::TracedTask<F> as core::future::future::Future>::poll::{{closure}} at /home/petrosagg/projects/tasktrace/src/lib.rs:116:50
//!  └╼ taskdump::foo::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:18:11
//!     └╼ taskdump::bar::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:22:5
//!        └╼ <tokio::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/poll_fn.rs:58:9
//!           ├╼ taskdump::bar::{{closure}}::{{closure}} at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/macros/join.rs:126:24
//!           │  └╼ <tokio::future::maybe_done::MaybeDone<Fut> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/maybe_done.rs:68:48
//!           │     └╼ taskdump::fiz::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:26:15
//!           │        └╼ taskdump::pending::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:14:8
//!           │           └╼ <core::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/future/poll_fn.rs:64:9
//!           │              └╼ taskdump::pending::{{closure}}::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:12:22
//!           │                 └╼ <core::task::wake::Waker as core::clone::Clone>::clone at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/task/wake.rs:342:29
//!           │                    └╼ tasktrace::clone_raw at /home/petrosagg/projects/tasktrace/src/lib.rs:135:5
//!           └╼ taskdump::bar::{{closure}}::{{closure}} at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/macros/join.rs:126:24
//!              └╼ <tokio::future::maybe_done::MaybeDone<Fut> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/maybe_done.rs:68:48
//!                 └╼ taskdump::buz::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:30:11
//!                    └╼ taskdump::baz::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:34:15
//!                       └╼ taskdump::pending::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:14:8
//!                          └╼ <core::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/future/poll_fn.rs:64:9
//!                             └╼ taskdump::pending::{{closure}}::{{closure}} at /home/petrosagg/projects/tasktrace/examples/taskdump.rs:12:22
//!                                └╼ <core::task::wake::Waker as core::clone::Clone>::clone at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/task/wake.rs:342:29
//!                                   └╼ tasktrace::clone_raw at /home/petrosagg/projects/tasktrace/src/lib.rs:135:5
//! ```
//!
//! ## How it works
//!
//! This crate exploits the fact that all leaf futures must clone the provided context Waker in
//! order to call it at some later time when progress can be made.
//!
//! When a traced future is requested to capture a callstack it wraps the normal waker into one
//! whose `Waker::clone` method is set up to capture a backtrace. Since only leaf futures ever
//! interact and clone the waker the stacktraces will necessarily contain the desired callstack.
//!
//! If multiple futures are being waited on (e.g through `select!`) then multiple stacktraces will
//! be captured for each polled future and their combined stacktrace will be displayed as a tree.

use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures_channel::oneshot::Sender;
use futures_core::Stream;
use pin_project_lite::pin_project;
use scoped_trace::Trace;

pub fn traced<F: Future>(fut: F) -> (TracedTask<F>, TraceHandle) {
    let (req_tx, req_rx) = futures_channel::mpsc::unbounded();
    let handle = TraceHandle { req_tx };
    let task = TracedTask { fut, req_rx };
    (task, handle)
}

pub struct TraceHandle {
    req_tx: UnboundedSender<TraceRequest>,
}

impl TraceHandle {
    pub async fn backtrace(&self) -> Option<Trace> {
        let (tx, rx) = futures_channel::oneshot::channel();
        self.req_tx.unbounded_send(TraceRequest(tx)).ok()?;
        rx.await.ok()
    }
}

struct TraceRequest(Sender<Trace>);

pin_project! {
    pub struct TracedTask<F> {
        #[pin]
        fut: F,
        #[pin]
        req_rx: UnboundedReceiver<TraceRequest>,
    }
}

impl<F: Future> Future for TracedTask<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Poll::Ready(Some(req)) = this.req_rx.poll_next(cx) {
            let trace_waker = TracedWaker(cx.waker());
            let raw_waker =
                RawWaker::new(&trace_waker as *const _ as *const (), &TRACE_WAKER_VTABLE);
            // SAFETY: RawWaker is well formed
            let waker = unsafe { ManuallyDrop::new(Waker::from_raw(raw_waker)) };
            let mut traced_cx = Context::from_waker(&waker);

            let (result, trace) = Trace::root(|| this.fut.poll(&mut traced_cx));
            let _ = req.0.send(trace);
            result
        } else {
            this.fut.poll(cx)
        }
    }
}

struct TracedWaker<'a>(&'a Waker);

const TRACE_WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone_raw, wake_raw, wake_by_ref_raw, drop_raw);

/// Clones a TracedWaker. Crucially, cloning converts it into the original wrapped Waker.
///
/// SAFETY:
/// The data pointer must point to a valid TracedWaker
unsafe fn clone_raw(data: *const ()) -> RawWaker {
    Trace::leaf();
    let this: &TracedWaker = &*(data as *const TracedWaker);
    let waker: Waker = this.0.clone();
    // SAFETY: Waker is a repr(transparent) over RawWaker
    std::mem::transmute::<Waker, RawWaker>(waker)
}

/// SAFETY:
/// The data pointer must point to a valid TracedWaker
unsafe fn wake_by_ref_raw(data: *const ()) {
    Trace::leaf();
    let this: &TracedWaker = &*(data as *const TracedWaker);
    this.0.wake_by_ref();
}

/// No one can end up holding an owned Waker that is backed by a TracedWaker since cloning the
/// original &Waker (backed by a TracedWaker) produces an owned Waker that is backed by the cloned
/// *inner* Waker. Therefore this method will never get called.
unsafe fn wake_raw(_data: *const ()) {
    unreachable!("an owned Waker backed by TracedWaker is never constructed");
}

/// No one can end up holding an owned Waker that is backed by a TracedWaker since cloning the
/// original &Waker (backed by a TracedWaker) produces an owned Waker that is backed by the cloned
/// *inner* Waker. Therefore this method will never get called.
unsafe fn drop_raw(_data: *const ()) {
    unreachable!("an owned Waker backed by TracedWaker is never constructed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_works() {
        let (foo_fut, trace_handle) = traced(foo());
        tokio::spawn(foo_fut);

        println!("{}", trace_handle.backtrace().await.unwrap());
    }

    async fn pending() {
        let mut waker = None;
        std::future::poll_fn(|cx| {
            waker = Some(cx.waker().clone());
            std::task::Poll::Pending
        })
        .await
    }

    async fn foo() {
        bar().await;
    }

    async fn bar() {
        tokio::join!(fiz(), buz());
    }

    async fn fiz() {
        pending().await;
    }

    async fn buz() {
        baz().await;
    }

    async fn baz() {
        pending().await;
    }
}
