use miopunch_native_probe::resources::{Resource, ResourceConfig, ResourceUse};
use miopunch_native_probe::{rtc_ice_profile::RtcIceProfile, rtc_transport::RtcTransport};

const SDP: &str = "v=0\r\na=ice-ufrag:test\r\na=candidate:1 1 udp 2122260223 10.203.1.2 42111 typ host\r\na=candidate:2 1 udp 1686052607 198.18.11.2 42111 typ srflx raddr 10.203.1.2 rport 42111\r\na=end-of-candidates\r\n";

#[test]
fn explicit_observer_profile_preserves_empty_defaults_and_excludes_private_candidates() {
    let empty = RtcTransport::new();
    assert!(empty.ice_profile().stun_servers.is_empty());
    assert_eq!(empty.ice_profile().local_sdp(SDP).unwrap(), SDP);
    let profile = RtcIceProfile {
        stun_servers: vec!["stun:198.18.0.10:3478".into()],
        candidate_addresses: vec![
            "198.18.11.2".parse().unwrap(),
            "198.18.12.2".parse().unwrap(),
        ],
        ..Default::default()
    };
    let transport = RtcTransport::with_ice_profile(profile).unwrap();
    let published = transport.ice_profile().local_sdp(SDP).unwrap();
    assert!(!published.contains("typ host"));
    assert!(published.contains("198.18.11.2 42111 typ srflx"));
    assert!(published.contains("raddr 10.203.1.2 rport 42111"));
    assert!(published.contains("a=ice-ufrag:test\r\n"));
    transport
        .ice_profile()
        .check_remote_sdp(&published)
        .unwrap();
    assert!(transport.ice_profile().check_remote_sdp(SDP).is_err());
    assert!(
        transport
            .ice_profile()
            .check_allowed(&["stun:198.18.0.10:3478".into()])
            .is_err()
    );
}

#[test]
fn observer_profile_refuses_turn_credentials_unbounded_or_relay_candidates() {
    let mut profile = RtcIceProfile::default();
    for url in [
        "turn:198.18.0.10:3478",
        "stun:user@198.18.0.10:3478",
        "stun:198.18.0.10:0",
        "stun:public.example:3478",
    ] {
        profile.stun_servers = vec![url.into()];
        assert!(
            RtcTransport::with_ice_profile(profile.clone()).is_err(),
            "{url}"
        );
    }
    profile.stun_servers = vec!["stun:198.18.0.10:3478".into(); 3];
    assert!(RtcTransport::with_ice_profile(profile).is_err());
    let relay = SDP.replace("typ srflx", "typ relay");
    assert!(RtcIceProfile::default().check_remote_sdp(&relay).is_err());
}

#[test]
fn explicit_observer_uses_existing_resource_identity_and_cannot_bypass_disabled_aliases() {
    let profile = RtcIceProfile {
        stun_servers: vec!["stun:198.18.0.10:3478".into()],
        candidate_addresses: vec![],
        ..Default::default()
    };
    assert_eq!(
        profile.observation_resources().unwrap(),
        ["stun://198.18.0.10:3478/".to_string()]
    );
    let mut configuration = ResourceConfig::default();
    configuration
        .import_trusted_replacement(vec![Resource {
            url: "stun://198.18.0.10:3478/".into(),
            uses: [ResourceUse::Observation].into(),
            limitation: "owned observation component".into(),
        }])
        .unwrap();
    assert!(configuration.allows("stun://198.18.0.10:3478/", ResourceUse::Observation));
    assert!(
        profile
            .check_allowed(&["stun://198.18.0.10:3478/".into()])
            .is_err()
    );
    assert!(
        profile
            .check_allowed(&["stun://198.18.0.10/ignored-path".into()])
            .is_err()
    );
}

#[test]
fn native_socket_bindings_are_bounded_and_do_not_restrict_published_candidates() {
    let mut profile = RtcIceProfile {
        local_bind_addresses: vec!["10.203.1.2".parse().unwrap()],
        ..Default::default()
    };
    profile.validate().unwrap();
    assert_eq!(profile.local_sdp(SDP).unwrap(), SDP);
    profile.check_remote_sdp(SDP).unwrap();
    for address in ["0.0.0.0", "127.0.0.1", "169.254.1.1", "224.0.0.1", "::1"] {
        profile.local_bind_addresses = vec![address.parse().unwrap()];
        assert!(profile.validate().is_err(), "{address}");
    }
    profile.local_bind_addresses = vec!["10.203.1.2".parse().unwrap(); 2];
    assert!(profile.validate().is_err());
    profile.local_bind_addresses = (1..=9)
        .map(|last| std::net::IpAddr::from([10, 203, 1, last]))
        .collect();
    assert!(profile.validate().is_err());
    let old = r#"{"stun_servers":[],"candidate_addresses":[]}"#;
    assert!(
        serde_json::from_str::<RtcIceProfile>(old)
            .unwrap()
            .local_bind_addresses
            .is_empty()
    );
}
