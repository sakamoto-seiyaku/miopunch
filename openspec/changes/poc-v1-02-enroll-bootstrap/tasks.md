## Done

- Freeze `InviteCapability` as MPINV1 with the fixed v1 field set, including `invite_id`.
- Freeze `join_request -> approve -> enroll_response` boundaries, including reply-topic pre-subscribe and `msg_id` dedupe.
- Limit persistence handoff to `MemberCredential + mailbox_secret + broker` written through `poc-v1-06-persistence`.
