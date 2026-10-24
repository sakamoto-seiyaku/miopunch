//! Public signed-ledger projection only. Fresh in-memory fixture identities;
//! no transport, OS clipboard, old credentials, capsules or clock manipulation.
use miopunch_native_probe::{
    management_auth::{ManagementAuth, ManagementExtension, NetworkAnchor},
    service_policy::{self, Service, ServiceAccess, ServiceAction, ServiceLedger, ServiceRule},
};
use p2panda_auth::{
    Access, GroupsExtensionArgs,
    group::{GroupAction, GroupMember},
};
use p2panda_core::{Hash, Header, SigningKey, VerifyingKey};

struct Fixture {
    issuer: SigningKey,
    other_provider: SigningKey,
    guest: SigningKey,
    anchor: NetworkAnchor,
    auth: ManagementAuth,
    ledger: ServiceLedger,
    observed: u64,
}
impl Fixture {
    fn new() -> Self {
        let root = SigningKey::generate();
        let issuer = SigningKey::generate();
        let other_provider = SigningKey::generate();
        let header = Header::builder().build(
            &root,
            ManagementExtension {
                version: 1,
                log_id: [0; 16],
                group: GroupsExtensionArgs {
                    group_id: root.verifying_key(),
                    dependencies: vec![],
                    action: GroupAction::Create {
                        initial_members: vec![
                            (
                                GroupMember::Individual(root.verifying_key()),
                                Access::manage(),
                            ),
                            (
                                GroupMember::Individual(issuer.verifying_key()),
                                Access::manage(),
                            ),
                        ],
                    },
                },
            },
        );
        let anchor = NetworkAnchor {
            root: root.verifying_key(),
            genesis: header.hash(),
        };
        let mut auth = ManagementAuth::new(anchor);
        auth.accept(&header.encode()).unwrap();
        // Ordinary members are admitted through Add, not the creator's
        // administrator-only initial Create contract.
        let admitted = Header::builder().build(
            &issuer,
            ManagementExtension {
                version: 1,
                log_id: *SigningKey::generate()
                    .as_bytes()
                    .first_chunk::<16>()
                    .unwrap(),
                group: GroupsExtensionArgs {
                    group_id: anchor.root,
                    dependencies: auth.tips(),
                    action: GroupAction::Add {
                        member: GroupMember::Individual(other_provider.verifying_key()),
                        access: Access::read(),
                    },
                },
            },
        );
        auth.accept(&admitted.encode()).unwrap();
        Self {
            issuer,
            other_provider,
            guest: SigningKey::generate(),
            anchor,
            auth,
            ledger: ServiceLedger::default(),
            observed: service_policy::now(),
        }
    }
    fn append(&mut self, action: ServiceAction) -> Hash {
        let record = self
            .ledger
            .author(self.anchor, &self.auth, &self.issuer, action)
            .unwrap();
        let id = Hash::digest(&record);
        self.ledger = self
            .ledger
            .merge(self.anchor, &self.auth, &[record])
            .unwrap();
        id
    }
    fn grant(&mut self, provider: VerifyingKey, service: Service, expires: u64) -> Hash {
        let token = *SigningKey::generate().as_bytes();
        let id = self.append(ServiceAction::InviteGuest {
            provider,
            service,
            token_hash: Hash::digest(token),
            created: self.observed - 120,
            expires,
        });
        self.append(ServiceAction::BindGuest {
            invitation: id,
            guest: self.guest.verifying_key(),
        });
        id
    }
    fn pairs(&self, local: VerifyingKey, service: Service) -> Vec<([u8; 32], u64)> {
        ServiceAccess {
            local,
            auth: self.auth.clone(),
            ledger: self.ledger.clone(),
        }
        .guest_pairs_for(service)
    }
    fn expect_both_roles(&self, service: Service, expires: Option<u64>) {
        let provider = self.issuer.verifying_key();
        let guest = self.guest.verifying_key();
        assert_eq!(
            self.pairs(provider, service),
            expires
                .map(|deadline| vec![(*guest.as_bytes(), deadline)])
                .unwrap_or_default()
        );
        assert_eq!(
            self.pairs(guest, service),
            expires
                .map(|deadline| vec![(*provider.as_bytes(), deadline)])
                .unwrap_or_default()
        );
    }
}

#[test]
fn clipboard_short_grant_cannot_borrow_longer_files_deadline() {
    let mut fixture = Fixture::new();
    let provider = fixture.issuer.verifying_key();
    let short = fixture.observed + 120;
    let long = fixture.observed + 600;
    fixture.grant(provider, Service::Clipboard, short);
    fixture.grant(provider, Service::Files, long);
    fixture.expect_both_roles(Service::Clipboard, Some(short));
    fixture.expect_both_roles(Service::Files, Some(long));
}

#[test]
fn files_short_grant_cannot_borrow_longer_clipboard_deadline() {
    let mut fixture = Fixture::new();
    let provider = fixture.issuer.verifying_key();
    let short = fixture.observed + 120;
    let long = fixture.observed + 600;
    fixture.grant(provider, Service::Files, short);
    fixture.grant(provider, Service::Clipboard, long);
    fixture.expect_both_roles(Service::Files, Some(short));
    fixture.expect_both_roles(Service::Clipboard, Some(long));
}

#[test]
fn ended_longer_same_service_grant_cannot_extend_live_short_grant() {
    for service in [Service::Clipboard, Service::Files] {
        let mut fixture = Fixture::new();
        let provider = fixture.issuer.verifying_key();
        let short = fixture.observed + 120;
        let long = fixture.observed + 600;
        fixture.grant(provider, service, short);
        let ended = fixture.grant(provider, service, long);
        fixture.append(ServiceAction::EndGuest { invitation: ended });
        fixture.expect_both_roles(service, Some(short));
    }
}

#[test]
fn independently_live_same_service_grants_keep_the_longest_exact_deadline() {
    for service in [Service::Clipboard, Service::Files] {
        let mut fixture = Fixture::new();
        let provider = fixture.issuer.verifying_key();
        fixture.grant(provider, service, fixture.observed + 120);
        let long = fixture.observed + 600;
        fixture.grant(provider, service, long);
        fixture.expect_both_roles(service, Some(long));
    }
}

#[test]
fn ended_and_expired_grants_project_no_pair() {
    for service in [Service::Clipboard, Service::Files] {
        let mut fixture = Fixture::new();
        let provider = fixture.issuer.verifying_key();
        let ended = fixture.grant(provider, service, fixture.observed + 600);
        fixture.append(ServiceAction::EndGuest { invitation: ended });
        fixture.grant(provider, service, fixture.observed - 60);
        fixture.expect_both_roles(service, None);
    }
}

#[test]
fn current_denial_filters_only_the_requested_service_and_unrelated_local_has_no_pair() {
    let mut fixture = Fixture::new();
    let provider = fixture.issuer.verifying_key();
    let deadline = fixture.observed + 120;
    fixture.grant(provider, Service::Clipboard, deadline);
    fixture.grant(provider, Service::Files, deadline);
    let mut rule = ServiceRule::default();
    rule.denied.insert(fixture.guest.verifying_key());
    fixture.append(ServiceAction::SetRule {
        provider,
        service: Service::Clipboard,
        previous: None,
        rule,
    });
    fixture.expect_both_roles(Service::Clipboard, None);
    fixture.expect_both_roles(Service::Files, Some(deadline));
    assert!(
        fixture
            .pairs(fixture.other_provider.verifying_key(), Service::Files)
            .is_empty()
    );
}

#[test]
fn different_providers_do_not_share_a_deadline_for_the_same_guest() {
    let mut fixture = Fixture::new();
    let provider = fixture.issuer.verifying_key();
    let other = fixture.other_provider.verifying_key();
    let short = fixture.observed + 120;
    let long = fixture.observed + 600;
    fixture.grant(provider, Service::Clipboard, short);
    fixture.grant(other, Service::Clipboard, long);
    let mut expected = vec![(*provider.as_bytes(), short), (*other.as_bytes(), long)];
    expected.sort();
    assert_eq!(
        fixture.pairs(fixture.guest.verifying_key(), Service::Clipboard),
        expected
    );
}
