//! Read-only facts about this fixture process's live sockets. No marks or
//! routes are changed; unavailable facts remain unknown with their error.
use std::{net::SocketAddr, time::SystemTime};

pub(super) fn snapshot(
    phase: &str,
    bound_udp: &[SocketAddr],
    selected: Option<serde_json::Value>,
) -> serde_json::Value {
    let started = std::time::Instant::now();
    let uid_read = std::fs::read_to_string("/proc/self/status");
    let uid_line = uid_read
        .as_ref()
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with("Uid:")));
    let uid_values = uid_line.and_then(|line| {
        line.split_whitespace()
            .skip(1)
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()
            .filter(|values| values.len() == 4)
    });
    let sockets = sockets(bound_udp);
    serde_json::json!({"event":"native-owned-sockets","phase":phase,
        "unixMicros":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).ok().map(|value|value.as_micros()),
        "pid":std::process::id(),"uidLine":uid_line,"processUids":uid_values,
        "uidReadErrno":uid_read.as_ref().err().and_then(|error|error.raw_os_error()),
        "sockets":sockets,"selectedPath":selected,"snapshotElapsedMicros":started.elapsed().as_micros()})
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn sockets(bound_udp: &[SocketAddr]) -> serde_json::Value {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // dup accepts a raw descriptor and atomically either duplicates its
    // current file description or reports EBADF. Unlike a borrowed-FD wrapper,
    // this makes no claim that an asynchronous SDK task cannot close the source.
    unsafe extern "C" {
        fn dup(fd: std::ffi::c_int) -> std::ffi::c_int;
    }
    let entries = match std::fs::read_dir("/proc/self/fd") {
        Ok(entries) => entries,
        Err(error) => {
            return serde_json::json!({"observed":false,"errno":error.raw_os_error(),"entries":null});
        }
    };
    // Snapshot names first, so descriptors created for inspection are not
    // recursively inspected as new SDK sockets.
    let mut readable = Vec::new();
    let mut unknown = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                unknown.push(serde_json::json!({"fd":null,"errno":error.raw_os_error()}));
                continue;
            }
        };
        let Some(fd) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let target = match std::fs::read_link(entry.path()) {
            Ok(target) => target,
            Err(error) => {
                unknown.push(serde_json::json!({"fd":fd,"errno":error.raw_os_error(),"socketKindKnown":false}));
                continue;
            }
        };
        if let Some(target) = target
            .to_str()
            .filter(|value| value.starts_with("socket:["))
        {
            readable.push((fd, target.to_owned()));
        }
    }
    let mut output = Vec::new();
    for (fd, original) in readable {
        // SAFETY: the C ABI takes an integer and handles a concurrently closed
        // descriptor as an OS error. The returned nonnegative descriptor is a
        // new owned reference, not the SDK's descriptor.
        let copied = unsafe { dup(fd) };
        if copied < 0 {
            output.push(
                serde_json::json!({"fd":fd,"inode":original,"dupVerified":false,
                "dupErrno":std::io::Error::last_os_error().raw_os_error()}),
            );
            continue;
        }
        // SAFETY: successful dup transfers this newly allocated descriptor to
        // us. OwnedFd closes it on every return/continue below.
        let copied = unsafe { OwnedFd::from_raw_fd(copied) };
        let cloexec = rustix::io::fcntl_setfd(&copied, rustix::io::FdFlags::CLOEXEC);
        let copy_target = std::fs::read_link(format!("/proc/self/fd/{}", copied.as_raw_fd()));
        let verified = copy_target
            .as_ref()
            .ok()
            .is_some_and(|target| target.to_str() == Some(original.as_str()));
        if !verified {
            output.push(serde_json::json!({"fd":fd,"inode":original,"dupVerified":false,
                "sourceChangedBeforeDup":true,"copyReadErrno":copy_target.err().and_then(|error|error.raw_os_error())}));
            continue;
        }
        let socket = socket2::SockRef::from(&copied);
        let kind = socket.r#type();
        let local = socket.local_addr();
        let peer = socket.peer_addr();
        let mark = socket.mark();
        let local_tuple = local.as_ref().ok().and_then(socket2::SockAddr::as_socket);
        let peer_tuple = peer.as_ref().ok().and_then(socket2::SockAddr::as_socket);
        let protocol = match kind.as_ref() {
            Ok(kind) if *kind == socket2::Type::DGRAM && local_tuple.is_some() => Some("udp"),
            Ok(kind) if *kind == socket2::Type::STREAM && local_tuple.is_some() => Some("tcp"),
            _ => None,
        };
        output.push(serde_json::json!({"fd":fd,"inode":original,"dupVerified":true,
            "cloexecSet":cloexec.is_ok(),"protocol":protocol,
            "localTuple":local_tuple.map(tuple),"localLookupErrno":local.err().and_then(|error|error.raw_os_error()),
            "peerTuple":peer_tuple.map(tuple),"peerLookupErrno":peer.err().and_then(|error|error.raw_os_error()),
            "endpointUdp":protocol==Some("udp")&&local_tuple.is_some_and(|local|bound_udp.contains(&local)),
            "soMark":{"operation":"getsockopt(SO_MARK)","observed":mark.is_ok(),"value":mark.as_ref().ok(),
                "errno":mark.err().and_then(|error|error.raw_os_error())},
            "socketUid":{"observed":false,"value":null}}));
    }
    serde_json::json!({"observed":true,"entries":output,"unclassifiedDescriptors":unknown})
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn sockets(_: &[SocketAddr]) -> serde_json::Value {
    serde_json::json!({"observed":false,"reason":"platform unsupported","entries":null})
}

fn tuple(address: SocketAddr) -> serde_json::Value {
    serde_json::json!({"address":address.ip().to_string(),"port":address.port(),
        "family":if address.is_ipv4(){"ipv4"}else{"ipv6"}})
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn actual_owned_udp_mark_and_connected_tcp_tuples_are_observed_without_retaining_sockets() {
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let udp_address = udp.local_addr().unwrap();
        let expected_mark = socket2::SockRef::from(&udp).mark();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let receipt = snapshot("local-test", &[udp_address], None);
        assert_eq!(receipt["pid"], std::process::id());
        assert_eq!(receipt["processUids"].as_array().unwrap().len(), 4);
        let entries = receipt["sockets"]["entries"].as_array().unwrap();
        let observed_udp = entries
            .iter()
            .find(|entry| entry["endpointUdp"] == true)
            .unwrap();
        assert_eq!(observed_udp["localTuple"], tuple(udp_address));
        match expected_mark {
            Ok(mark) => {
                assert_eq!(observed_udp["soMark"]["observed"], true);
                assert_eq!(observed_udp["soMark"]["value"], mark);
            }
            Err(error) => {
                // Sandboxes may deny even this read-only option. Preserve an
                // actual denial instead of turning it into an invented zero.
                assert_eq!(observed_udp["soMark"]["observed"], false);
                assert_eq!(observed_udp["soMark"]["value"], serde_json::Value::Null);
                assert_eq!(
                    observed_udp["soMark"]["errno"],
                    error.raw_os_error().unwrap()
                );
            }
        }
        assert_eq!(observed_udp["dupVerified"], true);
        assert!(entries.iter().any(|entry| {
            entry["protocol"] == "tcp"
                && entry["localTuple"] == tuple(client.local_addr().unwrap())
                && entry["peerTuple"] == tuple(client.peer_addr().unwrap())
                && entry["dupVerified"] == true
        }));
        drop((udp, client, server, listener));
        // Keeping the JSON receipt alive must not keep a copied socket alive.
        let rebound = std::net::UdpSocket::bind(udp_address).unwrap();
        drop((rebound, receipt));
    }
}
