//! TCP-only first contact through the production client and two NAT gateways.

#[tokio::test]
#[ignore = "requires the isolated two-NAT lab"]
async fn restricted_contact_delivers_the_first_native_reply_through_tcp() {
    super::native_service_nat::restricted_contact_first_reply(true).await;
}
