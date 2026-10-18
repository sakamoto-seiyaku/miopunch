#![cfg(not(target_arch = "wasm32"))]

// Keep these consumers byte-for-byte identical to the standalone programs run
// against the fixed official sources. The wrappers only register them with Cargo.
mod early_data {
    include!("ordinary_endpoint_consumers/early_data.rs");

    pub fn check() {
        main();
    }
}

mod routes {
    include!("ordinary_endpoint_consumers/routes.rs");

    pub fn check() {
        main();
    }
}

#[test]
fn original_consumer_exchanges_incoming_early_data_and_resumed_outgoing_zero_rtt() {
    early_data::check();
}

#[test]
fn original_routes_hooks_public_structs_and_required_socket_methods_remain_usable() {
    routes::check();
}
