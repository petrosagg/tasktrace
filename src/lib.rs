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
    use std::time::Duration;

    #[tokio::test]
    async fn it_works() {
        // First spawn a task that will get stuck
        let (task, handle) = traced(std::future::pending::<()>());
        tokio::task::spawn(task);
        println!("{}", handle.backtrace().await.unwrap());
    }

    #[inline(never)]
    async fn foo() {
        bar().await;
    }

    #[inline(never)]
    async fn bar() {
        baz().await;
    }

    #[inline(never)]
    async fn baz() {
        tokio::time::sleep(Duration::from_secs(u64::MAX)).await;
    }

    #[tokio::test]
    async fn it_works2() {
        let (task1, task1_trace) = traced(foo());
        let (task2, task2_trace) = traced(foo());
        tokio::select! {
            // run the following branches in order of their appearance
            biased;

            // spawn task #1
            _ = tokio::spawn(task1) => { unreachable!() }

            // spawn task #2
            _ = tokio::spawn(task2) => { unreachable!() }

            // print the running tasks
            _ = tokio::spawn(async {}) => {
                println!("{}", task1_trace.backtrace().await.unwrap());
                println!("{}", task2_trace.backtrace().await.unwrap());
            }
        }
    }
}
