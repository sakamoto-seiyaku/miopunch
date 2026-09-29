use std::io::{BufRead, BufReader};
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_miopunch-native-probe"))
        .args(["serve", mode])
        .arg(data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
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
