pub fn bypass(endpoint: &iroh::endpoint::RestrictedEndpoint) -> &iroh::Endpoint {
    endpoint.as_ref()
}
