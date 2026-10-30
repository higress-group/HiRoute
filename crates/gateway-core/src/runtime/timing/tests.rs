use super::*;
use std::time::Duration;

fn partition(snapshot: &TimingSnapshot) -> u64 {
    snapshot.local_us.iter().chain(&snapshot.wait_us).sum()
}

#[test]
fn exclusive_clock_accounts_phase_changes_and_pending_once() {
    let start = Instant::now();
    let clock = RequestClock::started_at(start);
    let mut state = clock.0.lock();
    state.settle(start + Duration::from_micros(100));
    state.phase = LocalPhase::Parse;
    state.settle(start + Duration::from_micros(300));
    state.active = false;
    state.pending = Some(WaitKind::UpstreamRead);
    state.settle(start + Duration::from_micros(10_300));
    state.active = true;
    state.phase = LocalPhase::Stream;
    state.settle(start + Duration::from_micros(10_350));
    state.active = false;
    state.pending = Some(WaitKind::DownstreamWrite);
    state.settle(start + Duration::from_micros(30_350));
    assert_eq!(state.local_ns[LocalPhase::Ingress as usize], 100_000);
    assert_eq!(state.local_ns[LocalPhase::Parse as usize], 200_000);
    assert_eq!(state.local_ns[LocalPhase::Stream as usize], 50_000);
    assert_eq!(state.wait_ns[WaitKind::UpstreamRead as usize], 10_000_000);
    assert_eq!(
        state.wait_ns[WaitKind::DownstreamWrite as usize],
        20_000_000
    );
    assert_eq!(
        state.local_ns.iter().chain(&state.wait_ns).sum::<u64>(),
        30_350_000
    );
}

#[tokio::test]
async fn nested_waits_are_leaf_owned_and_polling_remains_local() {
    let clock = RequestClock::new();
    clock
        .run(async {
            set_phase(LocalPhase::Stream);
            wait(WaitKind::Other, async {
                // Blocking here deliberately models synchronous decoder work.
                std::thread::sleep(Duration::from_millis(2));
                wait(
                    WaitKind::UpstreamRead,
                    tokio::time::sleep(Duration::from_millis(5)),
                )
                .await;
            })
            .await;
            wait(
                WaitKind::DownstreamWrite,
                tokio::time::sleep(Duration::from_millis(5)),
            )
            .await;
        })
        .await;
    let snapshot = clock.finish();
    assert_eq!(snapshot.elapsed_us, partition(&snapshot));
    assert!(snapshot.local_us[LocalPhase::Stream as usize] >= 2_000);
    assert!(snapshot.wait_us[WaitKind::UpstreamRead as usize] >= 5_000);
    assert!(snapshot.wait_us[WaitKind::DownstreamWrite as usize] >= 5_000);
    assert_eq!(snapshot.wait_us[WaitKind::Other as usize], 0);
    assert_eq!(clock.finish().elapsed_us, snapshot.elapsed_us);
}

#[tokio::test]
async fn decision_transport_waits_are_not_counted_again_as_model_waits() {
    let clock = RequestClock::new();
    clock
        .run(async {
            set_phase(LocalPhase::Decision);
            wait(
                WaitKind::Connect,
                tokio::time::sleep(Duration::from_millis(2)),
            )
            .await;
            wait(
                WaitKind::UpstreamRead,
                tokio::time::sleep(Duration::from_millis(2)),
            )
            .await;
        })
        .await;
    let snapshot = clock.finish();
    assert!(snapshot.wait_us[WaitKind::Decision as usize] >= 4_000);
    assert_eq!(snapshot.wait_us[WaitKind::Connect as usize], 0);
    assert_eq!(snapshot.wait_us[WaitKind::UpstreamRead as usize], 0);
    assert_eq!(partition(&snapshot), snapshot.elapsed_us);
}

#[tokio::test]
async fn concurrent_different_waits_remain_unclassified() {
    let clock = RequestClock::new();
    clock
        .run(async {
            tokio::join!(
                wait(
                    WaitKind::ClientRead,
                    tokio::time::sleep(Duration::from_millis(3))
                ),
                wait(
                    WaitKind::DownstreamWrite,
                    tokio::time::sleep(Duration::from_millis(3))
                ),
            );
        })
        .await;
    let snapshot = clock.finish();
    assert!(snapshot.wait_us[WaitKind::Other as usize] >= 3_000);
    assert_eq!(partition(&snapshot), snapshot.elapsed_us);
}

#[test]
fn dropped_pending_request_can_be_frozen_without_task_local_context() {
    use std::task::Context;
    let clock = RequestClock::new();
    let mut future =
        Box::pin(clock.run(wait(WaitKind::UpstreamRead, std::future::pending::<()>())));
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    assert!(future.as_mut().poll(&mut cx).is_pending());
    drop(future);
    let snapshot = clock.finish();
    assert_eq!(partition(&snapshot), snapshot.elapsed_us);
    assert_eq!(snapshot.local_us[LocalPhase::Stream as usize], 0);
}

#[tokio::test]
async fn dropped_pending_wait_does_not_label_later_local_wait() {
    use std::task::Poll;
    let clock = RequestClock::new();
    clock
        .run(async {
            // A timeout/select may poll an I/O future Pending, then choose another
            // branch in the same request poll and drop that I/O future.
            {
                let mut io = pin!(wait(WaitKind::UpstreamRead, std::future::pending::<()>()));
                poll_fn(|cx| {
                    assert!(io.as_mut().poll(cx).is_pending());
                    Poll::Ready(())
                })
                .await;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        })
        .await;
    let snapshot = clock.finish();
    assert_eq!(snapshot.wait_us[WaitKind::UpstreamRead as usize], 0);
    assert!(snapshot.wait_us[WaitKind::Other as usize] >= 5_000);
    assert_eq!(partition(&snapshot), snapshot.elapsed_us);
}
