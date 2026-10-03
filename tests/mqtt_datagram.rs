use miopunch_native_probe::mqtt_datagram::{MqttFacility, decode_datagram, encode_datagram};

#[test]
fn disabling_either_trusted_entry_disables_the_logical_facility() {
    let facility = MqttFacility::new(
        "owned-test-broker".into(),
        "mqtt://127.0.0.1:1883/".into(),
        "wss://localhost:8084/mqtt".into(),
    )
    .unwrap();
    assert!(facility.check_allowed(&[]).is_ok());
    assert!(
        facility
            .check_allowed(&[facility.native_url.clone()])
            .is_err()
    );
    assert!(facility.check_allowed(&[facility.wss_url.clone()]).is_err());
}

#[test]
fn mio1_is_shared_without_changing_the_existing_wire_envelope() {
    let source = [7; 16];
    let wire = b"MIO1\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07abc";
    assert_eq!(encode_datagram(source, b"abc").unwrap(), wire);
    assert_eq!(decode_datagram(wire).unwrap(), (source, b"abc".as_slice()));
    assert!(encode_datagram(source, &[0; 2048]).is_ok());
    assert!(encode_datagram(source, &[0; 2049]).is_err());
    assert!(decode_datagram(b"MIO1").is_err());
    assert!(
        decode_datagram(b"MIO2\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07\x07abc")
            .is_err()
    );
}

#[tokio::test]
async fn retiring_a_binding_cannot_write_the_next_generations_diagnostics() {
    use miopunch_native_probe::mqtt_transport::{MqttTransport, Traffic};
    use std::sync::Arc;

    let Ok(broker) = std::env::var("MIOPUNCH_MQTT_BROKER") else {
        return;
    };
    let traffic = Arc::new(Traffic::default());
    let transport = MqttTransport::new(broker.parse().unwrap(), traffic.clone());
    let first = transport.bind_datagrams().unwrap();
    transport.wait_ready().await.unwrap();
    let first_route = transport.route().unwrap();
    assert_eq!(first.local_addresses().len(), 1);
    drop(first);
    // Rebind immediately, before the old driver's graceful drain finishes.
    let second = transport.bind_datagrams().unwrap();
    transport.wait_ready().await.unwrap();
    assert_ne!(first_route, transport.route().unwrap());
    assert_eq!(second.local_addresses().len(), 1);
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let report = traffic.snapshot();
    assert_eq!(report["generation"], 2);
    assert_eq!(report["protocol"]["withdraw_ms"], serde_json::Value::Null);
    assert_eq!(report["protocol"]["unsubscribe_sent"], false);
    assert_eq!(report["protocol"]["disconnect_sent"], false);
    assert_eq!(report["queued_packets"], 0);
    drop(second);
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
}
