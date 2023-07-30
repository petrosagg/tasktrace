# backtrace-waker

Logical 'stack' traces of async functions.

This crate gives you the ability to capture a stacktrace of a running async task.

## Usage

To use, wrap the future you're interested in (usually the top level one) with
`backtrace_waker::trace` and use the returned handle to request a backtrace at any time.

```rust
#[tokio::main]
async fn main() {
    let (foo_fut, trace_handle) = backtrace_waker::traced(foo());
    tokio::spawn(foo_fut);

    println!("{}", trace_handle.backtrace().await.unwrap());
}

async fn pending() {
    let mut waker = None;
    std::future::poll_fn(|cx| {
        // Pretend to stash the waker for some future time like a real leaf Future would do.
        // This is what records the stacktrace.
        waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }).await
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
```

This example program will print out something along the lines of:

```text
╼ <backtrace_waker::TracedTask<F> as core::future::future::Future>::poll::{{closure}} at /home/petrosagg/projects/backtrace-waker/src/lib.rs:116:50
 └╼ taskdump::foo::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:18:11
    └╼ taskdump::bar::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:22:5
       └╼ <tokio::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/poll_fn.rs:58:9
          ├╼ taskdump::bar::{{closure}}::{{closure}} at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/macros/join.rs:126:24
          │  └╼ <tokio::future::maybe_done::MaybeDone<Fut> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/maybe_done.rs:68:48
          │     └╼ taskdump::fiz::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:26:15
          │        └╼ taskdump::pending::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:14:8
          │           └╼ <core::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/future/poll_fn.rs:64:9
          │              └╼ taskdump::pending::{{closure}}::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:12:22
          │                 └╼ <core::task::wake::Waker as core::clone::Clone>::clone at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/task/wake.rs:342:29
          │                    └╼ backtrace_waker::clone_raw at /home/petrosagg/projects/backtrace-waker/src/lib.rs:135:5
          └╼ taskdump::bar::{{closure}}::{{closure}} at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/macros/join.rs:126:24
             └╼ <tokio::future::maybe_done::MaybeDone<Fut> as core::future::future::Future>::poll at /home/petrosagg/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.29.1/src/future/maybe_done.rs:68:48
                └╼ taskdump::buz::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:30:11
                   └╼ taskdump::baz::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:34:15
                      └╼ taskdump::pending::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:14:8
                         └╼ <core::future::poll_fn::PollFn<F> as core::future::future::Future>::poll at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/future/poll_fn.rs:64:9
                            └╼ taskdump::pending::{{closure}}::{{closure}} at /home/petrosagg/projects/backtrace-waker/examples/taskdump.rs:12:22
                               └╼ <core::task::wake::Waker as core::clone::Clone>::clone at /rustc/8ede3aae28fe6e4d52b38157d7bfe0d3bceef225/library/core/src/task/wake.rs:342:29
                                  └╼ backtrace_waker::clone_raw at /home/petrosagg/projects/backtrace-waker/src/lib.rs:135:5
```

## How it works

This crate exploits the fact that all leaf futures must clone the provided context Waker in
order to call it at some later time when progress can be made.

When a traced future is requested to capture a callstack it wraps the normal waker into one
whose `Waker::clone` method is set up to capture a backtrace. Since only leaf futures ever
interact and clone the waker the stacktraces will necessarily contain the desired callstack.

If multiple futures are being waited on (e.g through `select!`) then multiple stacktraces will
be captured for each polled future and their combined stacktrace will be displayed as a tree.
