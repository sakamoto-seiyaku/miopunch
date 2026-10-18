pub fn bypass(router: &iroh::protocol::RestrictedRouter) -> &iroh::Endpoint {
    router.endpoint()
}
