use std::future::Future;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use pin_project_lite::pin_project;
use scoped_trace::Trace;

static COLLECT_TRACE: AtomicBool = AtomicBool::new(false);

pub fn traced<F: Future>(fut: F) -> (TracedTask<F>, TraceHandle) {
    let slot = Arc::new(Mutex::new(None));
    let waker = Arc::new(Mutex::new(None));
    let handle = TraceHandle {
        slot: Arc::clone(&slot),
        waker: Arc::clone(&waker),
    };

    let task = TracedTask { fut, slot, waker };

    (task, handle)
}

pub fn enable_tracing() {
    COLLECT_TRACE.store(true, Ordering::Release);
}

pub fn disable_tracing() {
    COLLECT_TRACE.store(true, Ordering::Release);
}

pub struct TraceHandle {
    slot: Arc<Mutex<Option<Trace>>>,
    waker: Arc<Mutex<Option<Waker>>>,
}

impl TraceHandle {
    pub fn poke(&self) {
        let waker = self.waker.lock().unwrap();
        if let Some(waker) = &*waker {
            waker.wake_by_ref();
        }
    }

    pub fn backtrace(&self) -> Option<Trace> {
        self.slot.lock().unwrap().take()
    }
}

pin_project! {
    pub struct TracedTask<F> {
        #[pin]
        fut: F,
        slot: Arc<Mutex<Option<Trace>>>,
        waker: Arc<Mutex<Option<Waker>>>,
    }
}

impl<F: Future> Future for TracedTask<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if COLLECT_TRACE.load(Ordering::Acquire) {
            let trace_waker = TraceWaker {
                inner: cx.waker(),
                slot: &this.slot,
            };
            let raw_waker =
                RawWaker::new(&trace_waker as *const _ as *const (), &TRACE_WAKER_VTABLE);
            // SAFETY: RawWaker is well formed
            let waker = unsafe { ManuallyDrop::new(Waker::from_raw(raw_waker)) };
            let mut traced_cx = Context::from_waker(&waker);

            let (result, trace) = Trace::root(|| this.fut.poll(&mut traced_cx));
            let mut slot = this.slot.lock().unwrap();
            *slot = Some(trace);
            result
        } else {
            // In the case where traces are not being collected we will just stash the current
            // waker in order to cause a wake up in the future when traces are needed.
            {
                let mut waker = this.waker.lock().unwrap();
                *waker = Some(cx.waker().clone());
            }

            this.fut.poll(cx)
        }
    }
}

struct TraceWaker<'a> {
    inner: &'a Waker,
    slot: &'a Mutex<Option<Trace>>,
}

const TRACE_WAKER_VTABLE: RawWakerVTable =
    RawWakerVTable::new(clone_raw, wake_raw, wake_by_ref_raw, drop_raw);

/// Clones a TraceWaker . Crucially, cloning converts it into the original wrapped
///
/// SAFETY:
/// The data pointer must point to a valid TraceWaker
unsafe fn clone_raw(data: *const ()) -> RawWaker {
    Trace::leaf();

    let this: &TraceWaker = &*(data as *const TraceWaker);
    let waker: Waker = this.inner.clone();
    // SAFETY: Waker is a repr(transparent) over RawWaker
    std::mem::transmute::<Waker, RawWaker>(waker)
}

/// SAFETY:
/// The data pointer must point to a valid TraceWaker
unsafe fn wake_by_ref_raw(data: *const ()) {
    Trace::leaf();
    let this: &TraceWaker = &*(data as *const TraceWaker);
    this.inner.wake_by_ref();
}

/// No one can end up holding an owned Waker that is backed by a TraceWaker since cloning the
/// original &Waker (backed by a TraceWaker) produces an owned Waker that is backed by the cloned
/// *inner* Waker. Therefore this method will never get called.
unsafe fn wake_raw(_data: *const ()) {
    unreachable!("an owned Waker backed by TraceWaker is never constructed");
}

/// No one can end up holding an owned Waker that is backed by a TraceWaker since cloning the
/// original &Waker (backed by a TraceWaker) produces an owned Waker that is backed by the cloned
/// *inner* Waker. Therefore this method will never get called.
unsafe fn drop_raw(_data: *const ()) {
    unreachable!("an owned Waker backed by TraceWaker is never constructed");
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

        // Verify that we don't have any backtraces yet
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(handle.backtrace().is_none());

        // Enable tracing and verify that we still don't have a backtrace
        enable_tracing();
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(handle.backtrace().is_none());

        // Poke the task which should now collect a backtrace
        handle.poke();
        tokio::time::sleep(Duration::from_millis(10)).await;
        println!("{}", handle.backtrace().unwrap());
    }
}
