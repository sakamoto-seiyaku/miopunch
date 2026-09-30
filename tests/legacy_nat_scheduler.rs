//! Observable legacy phase planning and real scheduler behavior.
use miopunch_native_probe::{
    nat_scheduler::{PhasePlan, RandomPortSampler},
    nat_selector::{Behavior, Role},
};
fn behavior(role: Role, radius: u16) -> Behavior {
    Behavior {
        role,
        ttl: 7,
        send_delay_ms: 2000,
        ports_range: radius,
        ports_random: 0,
        listen_random: 0,
    }
}

#[test]
fn random_sampling_keeps_legacy_bounds_ten_draw_skip_and_per_call_uniqueness() {
    let mut sampler = RandomPortSampler::default();
    assert_eq!(
        sampler.sample(|bounds| {
            assert_eq!(bounds, 1024..65535);
            1024
        }),
        Some(1024)
    );
    assert_eq!(sampler.sample(|_| 1024), None);
    assert_eq!(sampler.draws, 11);
    assert_eq!(sampler.skipped, 1);
    assert_eq!(sampler.sample(|_| 65534), Some(65534));
    let mut another_socket = RandomPortSampler::default();
    assert_eq!(another_socket.sample(|_| 1024), Some(1024));
    for _ in 0..997 {
        sampler.sample(|_| 65534);
    }
    assert_eq!(sampler.draws, 9982);
    assert_eq!(sampler.skipped, 998);
}

#[test]
fn random_listener_pair_has_the_original_thirty_second_read_extension() {
    let mut b = behavior(Role::Sender, 0);
    b.send_delay_ms = 3000;
    b.ports_random = 1000;
    let sender = PhasePlan::new(b, 0, &[], &[], 0);
    assert_eq!(sender.total_budget_ms, 38000);
    assert_eq!(sender.read_budget_ms, 35000);
    b.role = Role::Receiver;
    b.send_delay_ms = 0;
    b.ports_random = 0;
    b.listen_random = 256;
    let receiver = PhasePlan::new(b, 3000, &[], &[], 0);
    assert_eq!(receiver.total_budget_ms, 38000);
    assert_eq!(receiver.read_budget_ms, 38000);
}
#[test]
fn legacy_range_is_inclusive_and_clips_to_valid_udp_ports() {
    for (samples, diff, radius, expected) in [
        (vec!["192.0.2.1:41000"], 5, 10, (40990, 41010)),
        (vec!["192.0.2.1:41000"], 0, 10, (40995, 41005)),
        (vec!["192.0.2.1:41000"], 1, 10, (40994, 41006)),
        (vec!["192.0.2.1:41000"], 6, 10, (40990, 41010)),
        (vec!["192.0.2.1:41000"], 5, 2, (40998, 41002)),
        (vec!["192.0.2.1:1"], 5, 10, (1, 11)),
        (vec!["192.0.2.1:65535"], 5, 10, (65525, 65535)),
        (
            vec!["192.0.2.1:41003", "192.0.2.1:41001", "192.0.2.1:41002"],
            2,
            10,
            (40995, 41009),
        ),
    ] {
        let addresses = samples
            .into_iter()
            .map(|s| s.parse().unwrap())
            .collect::<Vec<_>>();
        let plan = PhasePlan::new(
            behavior(Role::Receiver, radius),
            10000,
            &addresses,
            &[],
            diff,
        );
        assert_eq!(plan.range, Some(expected));
        assert!(plan.explicit.is_empty());
        assert_eq!(plan.total_budget_ms, 15000);
        assert_eq!(plan.read_budget_ms, 13000);
    }
}
#[test]
fn sender_assisted_first_and_only_adjacent_duplicates_compact() {
    let a = "192.0.2.1:41000".parse().unwrap();
    let b = "192.0.2.2:41001".parse().unwrap();
    let plan = PhasePlan::new(behavior(Role::Sender, 0), 0, &[a, a, b, a], &[b, b, a], 0);
    assert_eq!(plan.explicit, vec![b, a, b, a]);
    assert_eq!(plan.candidates, vec![a, b, a]);
    assert_eq!(plan.range, None);
    assert_eq!(plan.read_budget_ms, 5000);
}
