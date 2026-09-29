use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(mode: &str, data_dir: &std::path::Path) -> (Server, String, String) {
    start_server_on(mode, data_dir, None)
}

fn start_server_on(
    mode: &str,
    data_dir: &std::path::Path,
    bind_ip: Option<&str>,
) -> (Server, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["serve", mode])
        .arg(data_dir)
        .args(bind_ip)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send((result, line));
    });
    let (result, line) = rx.recv_timeout(Duration::from_secs(15)).unwrap();
    result.unwrap();
    let fields: Vec<_> = line.trim().split(' ').collect();
    assert_eq!(fields.len(), 3, "server readiness: {line:?}");
    assert_eq!(fields[0], "READY");
    (Server(child), fields[1].into(), fields[2].into())
}

#[test]
fn wrapped_probe_accepts_a_wildcard_bind_for_a_lan_peer() {
    let root = tempfile::tempdir().unwrap();
    let (_server, server_id, address) =
        start_server_on("wrapped", &root.path().join("server"), Some("0.0.0.0"));
    let port = address.parse::<std::net::SocketAddr>().unwrap().port();
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["send", "wrapped"])
        .arg(root.path().join("client"))
        .arg(server_id)
        .arg(format!("127.0.0.1:{port}"))
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"PONG\n");
}

#[test]
fn invalid_mode_does_not_create_an_identity() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("unused");
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["serve", "invalid"])
        .arg(&data_dir)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!data_dir.exists());
}

fn assert_two_nodes_exchange_a_message(mode: &str) {
    let root = tempfile::tempdir().unwrap();
    let server_dir = root.path().join("server");
    let client_dir = root.path().join("client");
    let (_server, server_id, address) = start_server(mode, &server_dir);
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["send", mode])
        .arg(client_dir)
        .arg(server_id)
        .arg(address)
        .output()
        .unwrap();
    assert!(result.status.success(), "{:?}", result);
    assert_eq!(result.stdout, b"PONG\n");
}

#[test]
fn two_native_identities_exchange_a_message_over_udp() {
    assert_two_nodes_exchange_a_message("raw");
}

#[test]
fn p2panda_endpoint_reaches_iroh_lookup_without_an_address_book_entry() {
    assert_two_nodes_exchange_a_message("wrapped");
}

#[test]
fn two_native_identities_exchange_a_message_over_framed_tcp() {
    assert_two_nodes_exchange_a_message("tcp");
}

fn send_tcp_fault(addr: &str, frame: &[u8], half_close: bool) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut preface = [0; 6];
    stream.read_exact(&mut preface).unwrap();
    stream.write_all(&[127, 0, 0, 1, 0x40, 0x01]).unwrap();
    stream.write_all(frame).unwrap();
    if half_close {
        stream.shutdown(Shutdown::Write).unwrap();
    }
}

fn assert_tcp_fault_is_bounded(frame: &[u8], half_close: bool, expected_error: &str) {
    let root = tempfile::tempdir().unwrap();
    let (mut server, server_id, address) = start_server("tcp", &root.path().join("server"));
    send_tcp_fault(&address, frame, half_close);
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        server.0.try_wait().unwrap().is_none(),
        "malformed peer ended the server"
    );
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["send", "tcp"])
        .arg(root.path().join("client"))
        .arg(server_id)
        .arg(address)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    assert_eq!(result.stdout, b"PONG\n");
    assert!(server.0.wait().unwrap().success());
    let mut stderr = String::new();
    server
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.contains(expected_error), "{stderr}");
    let failed = stderr
        .lines()
        .find_map(|line| line.strip_prefix("TCP_TRAFFIC "))
        .and_then(|line| {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix("failed="))
        })
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap();
    assert!(failed >= 1, "{stderr}");
}

#[test]
fn truncated_tcp_frame_closes_without_reporting_echo_success() {
    assert_tcp_fault_is_bounded(&[0, 8, 1, 2, 3], true, "TCP_CLOSE result=early eof");
}

#[test]
fn oversized_tcp_frame_is_rejected_before_allocating_its_body() {
    assert_tcp_fault_is_bounded(&[8, 1], false, "invalid TCP frame length");
}

#[test]
fn sudden_tcp_disconnect_does_not_stop_future_connections() {
    assert_tcp_fault_is_bounded(&[0], false, "TCP_CLOSE result=early eof");
}

#[test]
fn wrong_target_identity_fails_on_reachable_tcp_carrier() {
    let root = tempfile::tempdir().unwrap();
    let (_real_server, real_id, address) = start_server("tcp", &root.path().join("real"));
    let (_other_server, wrong_id, _) = start_server("tcp", &root.path().join("other"));
    assert_ne!(real_id, wrong_id);
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["send", "tcp"])
        .arg(root.path().join("client"))
        .arg(wrong_id)
        .arg(address)
        .output()
        .unwrap();
    assert!(!result.status.success(), "{result:?}");
    assert!(!result.stdout.windows(4).any(|part| part == b"PONG"));
}

#[test]
fn wrong_target_identity_fails_even_at_a_reachable_udp_address() {
    let root = tempfile::tempdir().unwrap();
    let (_real_server, real_id, address) = start_server("wrapped", &root.path().join("real"));
    let (_other_server, wrong_id, _) = start_server("wrapped", &root.path().join("other"));
    assert_ne!(real_id, wrong_id);
    let result = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["send", "wrapped"])
        .arg(root.path().join("client"))
        .arg(wrong_id)
        .arg(address)
        .output()
        .unwrap();
    assert!(!result.status.success(), "{:?}", result);
    assert!(!result.stdout.windows(4).any(|part| part == b"PONG"));
}

#[test]
fn restarting_a_node_keeps_its_identity() {
    let root = tempfile::tempdir().unwrap();
    let data_dir = root.path().join("node");
    let (first, first_id, _) = start_server("wrapped", &data_dir);
    drop(first);
    let (_second, second_id, _) = start_server("wrapped", &data_dir);
    assert_eq!(first_id, second_id);
}
