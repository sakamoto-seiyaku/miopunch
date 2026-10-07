//! Scoring checks only, not SDK admission, RTC qualification or route selection.
//! Compile the actual private scorer with the repository's real dependencies;
//! no synthetic iroh PathSelectionContext grants a path to a connection.
#[path = "../src/path_selection.rs"]
mod path_selection;

// The source's existing native selector() defaults refer to actual carriers.
#[cfg(feature = "rtc-probe")]
use miopunch_native_probe::rtc_transport;
use miopunch_native_probe::{irc_transport, member_transport, mqtt_transport, tcp_transport};
use path_selection::{CarrierClasses, PathPreference, PathScore, PathSelectionPolicy};

use iroh::endpoint::transports::FourTuple;
use std::time::Duration;

const RTC: u64 = 0x6d696f727463;
const UNKNOWN: u64 = 0x756e6b6e6f776e;

fn custom(id: u64) -> FourTuple {
    FourTuple::Custom {
        remote: iroh_base::CustomAddr::from_parts(id, &[1]),
        local: None,
    }
}

fn ip() -> FourTuple {
    FourTuple::Ip {
        remote: "127.0.0.1:41000".parse().unwrap(),
        local: None,
    }
}

fn relay() -> FourTuple {
    FourTuple::Relay {
        url: "http://127.0.0.1:42000".parse().unwrap(),
        endpoint_id: iroh_base::SecretKey::from_bytes(&[1; 32]).public(),
    }
}

fn classes() -> CarrierClasses {
    CarrierClasses::new(
        [tcp_transport::DATA_TRANSPORT_ID, RTC],
        [member_transport::DATA_TRANSPORT_ID],
    )
}

fn score(path: &FourTuple, millis: u64, preference: PathPreference) -> PathScore {
    PathScore::new(path, Duration::from_millis(millis), preference, &classes())
}

#[test]
fn quality_keeps_all_carriers_equal_and_requires_five_milliseconds() {
    let current = score(&relay(), 20, PathPreference::Quality);
    for path in [ip(), custom(RTC), custom(tcp_transport::DATA_TRANSPORT_ID)] {
        assert!(!score(&path, 16, PathPreference::Quality).should_replace(Some(current)));
        assert!(score(&path, 15, PathPreference::Quality).should_replace(Some(current)));
        assert!(!score(&path, 20, PathPreference::Quality).should_replace(Some(current)));
        assert!(!score(&path, 21, PathPreference::Quality).should_replace(Some(current)));
        assert!(score(&path, 90, PathPreference::Quality).should_replace(None));
    }
    assert_eq!(
        score(&custom(RTC), 20, PathPreference::Quality),
        score(&relay(), 20, PathPreference::Quality)
    );
    assert!(
        score(&relay(), 15, PathPreference::Quality).should_replace(Some(score(
            &custom(RTC),
            20,
            PathPreference::Quality
        )))
    );
}

#[test]
fn direct_preference_includes_installed_rtc_ip_and_legacy_tcp() {
    let current_relay = score(&relay(), 1, PathPreference::PreferDirect);
    for path in [ip(), custom(RTC), custom(tcp_transport::DATA_TRANSPORT_ID)] {
        assert!(score(&path, 90, PathPreference::PreferDirect).should_replace(Some(current_relay)));
        assert!(
            !score(&relay(), 1, PathPreference::PreferDirect).should_replace(Some(score(
                &path,
                90,
                PathPreference::PreferDirect
            )))
        );
    }
    let current_ip = score(&ip(), 20, PathPreference::PreferDirect);
    assert!(
        !score(&custom(RTC), 16, PathPreference::PreferDirect).should_replace(Some(current_ip))
    );
    assert!(score(&custom(RTC), 15, PathPreference::PreferDirect).should_replace(Some(current_ip)));
}

#[test]
fn relay_preference_preserves_ordinary_and_explicit_member_data_classes() {
    let current_direct = score(&custom(RTC), 1, PathPreference::PreferRelay);
    for path in [relay(), custom(member_transport::DATA_TRANSPORT_ID)] {
        assert!(score(&path, 90, PathPreference::PreferRelay).should_replace(Some(current_direct)));
        assert!(
            !score(&custom(RTC), 1, PathPreference::PreferRelay).should_replace(Some(score(
                &path,
                90,
                PathPreference::PreferRelay
            )))
        );
    }
    let current_relay = score(&relay(), 20, PathPreference::PreferRelay);
    let member = custom(member_transport::DATA_TRANSPORT_ID);
    assert!(!score(&member, 16, PathPreference::PreferRelay).should_replace(Some(current_relay)));
    assert!(score(&member, 15, PathPreference::PreferRelay).should_replace(Some(current_relay)));
}

#[test]
fn signaling_unknown_and_uninstalled_rtc_receive_no_carrier_bias() {
    // Real signaling carrier IDs remain absent from the installed DATA classes.
    for signal in [
        member_transport::TRANSPORT_ID,
        mqtt_transport::TRANSPORT_ID,
        irc_transport::TRANSPORT_ID,
        UNKNOWN,
    ] {
        let path = custom(signal);
        assert!(
            !score(&path, 1, PathPreference::PreferDirect).should_replace(Some(score(
                &ip(),
                90,
                PathPreference::PreferDirect
            )))
        );
        assert!(
            !score(&path, 1, PathPreference::PreferRelay).should_replace(Some(score(
                &relay(),
                90,
                PathPreference::PreferRelay
            )))
        );
    }
    let uninstalled = CarrierClasses::default();
    let rtc = PathScore::new(
        &custom(RTC),
        Duration::from_millis(1),
        PathPreference::PreferDirect,
        &uninstalled,
    );
    let current = PathScore::new(
        &ip(),
        Duration::from_millis(90),
        PathPreference::PreferDirect,
        &uninstalled,
    );
    assert!(!rtc.should_replace(Some(current)));
}

#[test]
fn current_and_explicit_selector_apis_keep_interval_defaults_and_bounds() {
    let policy = PathSelectionPolicy::default();
    assert_eq!(policy.preference, PathPreference::Quality);
    assert_eq!(
        policy.selector().unwrap().reevaluation_interval(),
        Some(Duration::from_secs(1))
    );
    assert_eq!(
        policy
            .selector_with_carriers(classes())
            .unwrap()
            .reevaluation_interval(),
        Some(Duration::from_secs(1))
    );
    for interval in [249, 30_001] {
        let policy = PathSelectionPolicy {
            reevaluation_interval_ms: interval,
            ..policy.clone()
        };
        assert!(policy.selector().is_err());
        assert!(policy.selector_with_carriers(classes()).is_err());
    }
    for interval in [250, 30_000] {
        let policy = PathSelectionPolicy {
            reevaluation_interval_ms: interval,
            ..policy.clone()
        };
        assert_eq!(
            policy
                .selector_with_carriers(classes())
                .unwrap()
                .reevaluation_interval(),
            Some(Duration::from_millis(interval))
        );
    }
}

#[cfg(feature = "rtc-probe")]
#[test]
fn native_installed_carrier_classes_rank_rtc_without_biasing_uninstalled_or_signaling() {
    let score = |classes: &CarrierClasses, path: &FourTuple, ms, preference| {
        PathScore::new(path, Duration::from_millis(ms), preference, classes)
    };
    let installed = CarrierClasses::native(true);
    let uninstalled = CarrierClasses::native(false);
    let rtc = custom(rtc_transport::TRANSPORT_ID);
    let member = custom(member_transport::DATA_TRANSPORT_ID);
    let current = score(&installed, &relay(), 1, PathPreference::PreferDirect);
    assert!(
        score(&installed, &rtc, 90, PathPreference::PreferDirect).should_replace(Some(current))
    );
    assert!(
        !score(&uninstalled, &rtc, 90, PathPreference::PreferDirect).should_replace(Some(current))
    );
    assert!(
        !score(&installed, &member, 90, PathPreference::PreferDirect).should_replace(Some(current))
    );
    assert!(
        score(&installed, &member, 90, PathPreference::PreferRelay).should_replace(Some(score(
            &installed,
            &rtc,
            1,
            PathPreference::PreferRelay
        )))
    );
    for signal in [
        member_transport::TRANSPORT_ID,
        mqtt_transport::TRANSPORT_ID,
        irc_transport::TRANSPORT_ID,
    ] {
        assert!(
            !score(
                &installed,
                &custom(signal),
                90,
                PathPreference::PreferDirect
            )
            .should_replace(Some(current))
        );
    }
    let quality = score(&installed, &relay(), 20, PathPreference::Quality);
    assert!(!score(&installed, &rtc, 16, PathPreference::Quality).should_replace(Some(quality)));
    assert!(score(&installed, &rtc, 15, PathPreference::Quality).should_replace(Some(quality)));
}
