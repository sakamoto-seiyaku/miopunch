//! Stable request namespace shared by native and browser precode clients.
/// A new code is a new redemption; retries of that code and admission attempt
/// retain the same identity without exposing the bearer token on the wire ID.
pub fn precode_request_id(attempt: [u8; 16], token: [u8; 32]) -> [u8; 16] {
    let mut hash = blake3::Hasher::new_derive_key("miopunch precode redemption id v1");
    hash.update(&attempt);
    hash.update(&token);
    hash.finalize().as_bytes()[..16].try_into().unwrap()
}
