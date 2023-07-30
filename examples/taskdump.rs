#[tokio::main]
async fn main() {
    let (foo_fut, trace_handle) = tasktrace::traced(foo());
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
