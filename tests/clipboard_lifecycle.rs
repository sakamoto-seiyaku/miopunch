//! Public SDK cancellation semantics. This narrowly tests startup ownership,
//! not cross-desktop clipboard delivery (qualified by check_clipboard.py).
use miopunch_native_probe::{
    manual_admission::AdmissionStage, native_client::NativeClient, service_policy,
};
use std::{net::Ipv4Addr, path::PathBuf, sync::Arc, time::Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_an_in_flight_clipboard_start_cannot_create_a_late_pair() {
    let root = PathBuf::from(
        std::env::var_os("M117_CLIPBOARD_PRIVATE").expect("owned private test directory"),
    );
    let source = NativeClient::create(root.join("sdk-source"), Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    source
        .export_recovery_card_to(root.join("sdk-card"), None)
        .await
        .unwrap();
    source.acknowledge_recovery_saved().await.unwrap();
    let invitation = source
        .issue_precode(service_policy::now() + 3600, Some(1))
        .await
        .unwrap();
    let target = NativeClient::join(root.join("sdk-target"), &invitation, Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    assert_eq!(
        target.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    assert_eq!(source.enable_clipboard().await.unwrap()["enabled"], true);
    assert_eq!(target.enable_clipboard().await.unwrap()["enabled"], true);
    let source = Arc::new(source);
    let address = target.address();
    let (polled_pending, first_pending) = tokio::sync::oneshot::channel();
    let starting = {
        let source = source.clone();
        tokio::spawn(async move {
            let future = source.start_clipboard(address);
            tokio::pin!(future);
            let mut notice = Some(polled_pending);
            std::future::poll_fn(|context| {
                let result = std::future::Future::poll(future.as_mut(), context);
                if result.is_pending() {
                    if let Some(notice) = notice.take() {
                        let _ = notice.send(());
                    }
                }
                result
            })
            .await
        })
    };
    // Observe the public Future itself returning Pending. An executor yield or
    // an unfinished JoinHandle alone would only prove the task was queued.
    // No private SDK state, test clock, fake peer or packet injection is used.
    let pending = first_pending.await.is_ok() && !starting.is_finished();
    let stop_started = Instant::now();
    source.stop_clipboard().await;
    let stop_seconds = stop_started.elapsed().as_secs_f64();
    let stopped = source.clipboard_status().await.unwrap();
    let original_start = starting.await.unwrap();
    let after_original_start = source.clipboard_status().await.unwrap();
    let remained_stopped = stopped["peer"].is_null() && after_original_start["peer"].is_null();
    eprintln!(
        "public-start-stop pending={pending} start_succeeded={} remained_stopped={remained_stopped} stop_seconds={stop_seconds}",
        original_start.is_ok()
    );
    source.stop_clipboard().await;
    target.stop_clipboard().await;
    target.close().await.unwrap();
    Arc::try_unwrap(source)
        .ok()
        .expect("all public start owners joined")
        .close()
        .await
        .unwrap();
    assert!(
        pending,
        "actual pending-start prerequisite was not established"
    );
    assert!(
        remained_stopped,
        "the original public start created a late active pair after stop returned"
    );
}

async fn extra_pair(name: &str) -> (Arc<NativeClient>, NativeClient) {
    let root = PathBuf::from(std::env::var_os("M117_CLIPBOARD_PRIVATE").unwrap()).join(name);
    std::fs::create_dir(&root).unwrap();
    let source = NativeClient::create(root.join("source"), Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    source
        .export_recovery_card_to(root.join("card"), None)
        .await
        .unwrap();
    source.acknowledge_recovery_saved().await.unwrap();
    let invitation = source
        .issue_precode(service_policy::now() + 3600, Some(1))
        .await
        .unwrap();
    let target = NativeClient::join(root.join("target"), &invitation, Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    assert_eq!(
        target.request_admission(&invitation).await.unwrap(),
        AdmissionStage::Complete
    );
    source.enable_clipboard().await.unwrap();
    target.enable_clipboard().await.unwrap();
    (Arc::new(source), target)
}

async fn extra_pending(
    source: Arc<NativeClient>,
    address: iroh::EndpointAddr,
) -> (
    tokio::task::JoinHandle<
        Result<serde_json::Value, miopunch_native_probe::native_client::ClientError>,
    >,
    bool,
) {
    let (notice, pending) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let future = source.start_clipboard(address);
        tokio::pin!(future);
        let mut notice = Some(notice);
        std::future::poll_fn(|context| {
            let result = std::future::Future::poll(future.as_mut(), context);
            if result.is_pending() {
                if let Some(notice) = notice.take() {
                    let _ = notice.send(());
                }
            }
            result
        })
        .await
    });
    let observed = pending.await.is_ok() && !task.is_finished();
    (task, observed)
}

async fn extra_close(source: Arc<NativeClient>, target: NativeClient) {
    source.stop_clipboard().await;
    target.stop_clipboard().await;
    target.close().await.unwrap();
    Arc::try_unwrap(source)
        .ok()
        .expect("all starting futures joined")
        .close()
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_clipboard_extra_disable_retires_both_running_and_queued_starts() {
    let (source, target) = extra_pair("queued-disable").await;
    let (first, first_pending) = extra_pending(source.clone(), target.address()).await;
    let (second, second_pending) = extra_pending(source.clone(), target.address()).await;
    source.disable_clipboard().await.unwrap();
    let disabled = source.clipboard_status().await.unwrap();
    source.enable_clipboard().await.unwrap();
    let first_failed = first.await.unwrap().is_err();
    let second_failed = second.await.unwrap().is_err();
    let no_late_pair = source.clipboard_status().await.unwrap()["peer"].is_null();
    let fresh = source.start_clipboard(target.address()).await.unwrap();
    let explicitly_started = fresh["phase"] == "synchronizing";
    extra_close(source, target).await;
    assert!(
        first_pending && second_pending,
        "both public futures actually pending before disable"
    );
    assert!(!disabled["enabled"].as_bool().unwrap() && disabled["peer"].is_null());
    assert!(first_failed && second_failed && no_late_pair && explicitly_started);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_clipboard_extra_dropped_pending_future_releases_the_pair_reservation() {
    let (source, target) = extra_pair("dropped-start").await;
    let (starting, pending) = extra_pending(source.clone(), target.address()).await;
    let was_connecting = source.clipboard_status().await.unwrap()["phase"] == "connecting";
    // Normal caller cancellation of this owned public future, not an injected
    // transport fault. Joining it observes destruction of the setup owner.
    starting.abort();
    let cancelled = starting.await.unwrap_err().is_cancelled();
    let released = source.clipboard_status().await.unwrap()["peer"].is_null();
    let fresh = source.start_clipboard(target.address()).await.unwrap();
    let fresh_completed = fresh["phase"] == "synchronizing";
    extra_close(source, target).await;
    assert!(pending && was_connecting && cancelled);
    assert!(released && fresh_completed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_clipboard_extra_failed_setup_and_duplicate_peer_do_not_replace_the_pair() {
    let (source, target) = extra_pair("failed-and-duplicate").await;
    target.disable_clipboard().await.unwrap();
    let rejected = source.start_clipboard(target.address()).await.is_err();
    let released = source.clipboard_status().await.unwrap()["peer"].is_null();
    target.enable_clipboard().await.unwrap();
    let fresh = source.start_clipboard(target.address()).await.unwrap();
    let duplicate_failed = source.start_clipboard(target.address()).await.is_err();
    let same_peer = source.clipboard_status().await.unwrap()["peer"] == fresh["peer"];
    extra_close(source, target).await;
    assert!(rejected && released && duplicate_failed && same_peer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clipboard_guest_acl_recovery_observes_existing_public_contact_backoff() {
    use miopunch_native_probe::{
        resources::ResourceContactError,
        service_policy::{Service, ServiceRule},
    };
    use p2panda_core::VerifyingKey;
    use std::time::Duration;

    let root =
        PathBuf::from(std::env::var_os("M117_CLIPBOARD_PRIVATE").unwrap()).join("guest-backoff");
    let provider = NativeClient::create(root.join("provider"), Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    provider
        .export_recovery_card_to(root.join("card"), None)
        .await
        .unwrap();
    provider.acknowledge_recovery_saved().await.unwrap();
    provider.enable_clipboard().await.unwrap();
    let invitation = provider
        .issue_service_guest(
            provider.address(),
            Service::Clipboard,
            service_policy::now() + 90,
        )
        .await
        .unwrap();
    let guest = NativeClient::join_guest(root.join("guest"), &invitation, Ipv4Addr::UNSPECIFIED)
        .await
        .unwrap();
    guest.enable_clipboard().await.unwrap();
    guest.redeem_guest().await.unwrap();
    guest.start_clipboard(provider.address()).await.unwrap();
    let provider_id = VerifyingKey::from_bytes(provider.address().id.as_bytes()).unwrap();
    let guest_id = VerifyingKey::from_bytes(guest.address().id.as_bytes()).unwrap();
    let mut denied = ServiceRule::default();
    denied.denied.insert(guest_id);
    let revision = provider
        .set_service_rule(provider_id, Service::Clipboard, None, denied)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !guest.clipboard_status().await.unwrap()["peer"].is_null() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let denied_start = guest.start_clipboard(provider.address()).await.is_err();
    provider
        .set_service_rule(
            provider_id,
            Service::Clipboard,
            Some(revision),
            ServiceRule::default(),
        )
        .await
        .unwrap();
    let immediate = guest.redeem_guest().await;
    let remaining = immediate
        .as_ref()
        .err()
        .and_then(|failure| failure.downcast_ref::<ResourceContactError>())
        .and_then(|failure| match failure {
            ResourceContactError::TemporarilyUnreachable { retry_after, .. } => Some(*retry_after),
            ResourceContactError::Configuration(_) => None,
        });
    let bounded_backoff = remaining
        .is_some_and(|remaining| !remaining.is_zero() && remaining <= Duration::from_secs(1));
    let mut recovered = false;
    if let Some(remaining) = remaining.filter(|remaining| *remaining <= Duration::from_secs(1)) {
        // Respect the actual public retry-after, then make exactly one new
        // ordinary request. No blind retry, clock/transport injection or reset.
        tokio::time::sleep(remaining + Duration::from_millis(50)).await;
        recovered = guest.redeem_guest().await.is_ok()
            && guest.start_clipboard(provider.address()).await.is_ok();
    }
    eprintln!(
        "guest-acl-recovery denied_start={denied_start} actual_bounded_backoff={bounded_backoff} recovered={recovered}"
    );
    guest.stop_clipboard().await;
    provider.stop_clipboard().await;
    guest.close().await.unwrap();
    provider.close().await.unwrap();
    assert!(denied_start && bounded_backoff && recovered);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_member_acl_contacts_respect_public_shared_backoff() {
    use miopunch_native_probe::{
        resources::ResourceContactError,
        service_policy::{Service, ServiceRule},
    };
    use p2panda_core::VerifyingKey;
    use std::time::Duration;

    let (source, target) = extra_pair("member-acl-backoff").await;
    let provider = VerifyingKey::from_bytes(target.address().id.as_bytes()).unwrap();
    let caller = VerifyingKey::from_bytes(source.address().id.as_bytes()).unwrap();
    let mut rule = ServiceRule::default();
    rule.denied.insert(caller);
    target
        .set_service_rule(provider, Service::Clipboard, None, rule)
        .await
        .unwrap();
    let mut observations = Vec::new();
    for expected_bound in [1, 2] {
        let denied = source.start_clipboard(target.address()).await.is_err();
        let policy_contact = source.sync_services(target.address()).await;
        let remaining = policy_contact
            .as_ref()
            .err()
            .and_then(|failure| failure.downcast_ref::<ResourceContactError>())
            .and_then(|failure| match failure {
                ResourceContactError::TemporarilyUnreachable { retry_after, .. } => {
                    Some(*retry_after)
                }
                ResourceContactError::Configuration(_) => None,
            });
        let observed = denied
            && remaining.is_some_and(|delay| {
                !delay.is_zero() && delay <= Duration::from_secs(expected_bound)
            });
        observations.push(observed);
        eprintln!(
            "member-acl-backoff attempt_bound={expected_bound} actual_bounded_backoff={observed}"
        );
        if let Some(delay) = remaining.filter(|delay| *delay <= Duration::from_secs(expected_bound))
        {
            tokio::time::sleep(delay + Duration::from_millis(50)).await;
        }
    }
    // The current Clipboard denial does not prohibit the ordinary management
    // protocol. One request after its observed retry-after must still work.
    let policy_recovered = source.sync_services(target.address()).await.is_ok();
    extra_close(source, target).await;
    assert!(observations == [true, true] && policy_recovered);
}
